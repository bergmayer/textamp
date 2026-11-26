//! Main application event loop (musikcube-style).
//!
//! Handles input events, async task coordination, and state updates.

use crate::app::PlayStatus;
use crate::app::{AppState, Event};

use crate::ui;

use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, Event as CrosstermEvent, MouseEventKind};
use crossterm::{SynchronizedUpdate, execute};
use ratatui::prelude::*;
use std::io::Stdout;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
                        Ok(CrosstermEvent::Paste(text)) => Some(Event::Paste(text)),
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
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            tracing::warn!("Terminal input thread panicked during shutdown");
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
        state.station_nav.loading,
    ]
    .contains(&true);
    let popup_loading = [
        state
            .popups
            .artist_bio()
            .is_some_and(|popup| popup.task.is_some()),
        state
            .popups
            .adventure_launcher()
            .is_some_and(|popup| popup.loading),
    ]
    .contains(&true);

    [
        matches!(
            state.playback.status,
            PlayStatus::Playing | PlayStatus::Buffering
        ),
        marquee_active
            || state
                .hit_regions
                .header_clocks
                .iter()
                .any(|clock| clock.3.elapsed() >= crate::util::column_heading::DELAY),
        library_loading,
        popup_loading,
        state.similar.request.is_loading(),
        state.related.loading,
        state.artwork.fetch.is_loading(),
        !state.artwork.grid_pending.is_empty(),
        state.waveform.generating(),
        state.spectrogram.generating(),
        state.station_starting.is_some(),
        state.radio.refill != crate::app::state::RadioRefill::Idle,
        state.list_filter.loading,
        state.notifications.toast_show_time.is_some(),
        state.notifications.status_show_time.is_some(),
        state.alt_bar_until.is_some(),
    ]
    .contains(&true)
}

/// Terminal signal streams registered before entering raw mode.
pub struct ShutdownSignal {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(windows)]
    interrupt: tokio::signal::windows::CtrlC,
}
impl ShutdownSignal {
    /// Register process interruption handlers before terminal setup.
    /// # Errors
    /// Propagates operating-system signal registration failures.
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            #[cfg(unix)]
            interrupt: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
            #[cfg(unix)]
            terminate: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
            #[cfg(windows)]
            interrupt: tokio::signal::windows::ctrl_c()?,
        })
    }
    async fn recv(&mut self) {
        #[cfg(unix)]
        tokio::select! {_ = self.interrupt.recv() => {}, _ = self.terminate.recv() => {}}
        #[cfg(windows)]
        {
            self.interrupt.recv().await;
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

/// Main event loop.
pub struct EventLoop {
    session: crate::app::runtime::Session,
    signals: ShutdownSignal,
    shutdown: Arc<AtomicBool>,
}

impl EventLoop {
    /// Attach the external terminal adapter to an owned live session.
    pub fn new(session: crate::app::runtime::Session, signals: ShutdownSignal) -> Self {
        Self {
            session,
            signals,
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Run the main event loop.
    pub async fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        let tasks = self.session.task_context();
        let result = tasks.run(self.run_session(terminal)).await;
        drop(tasks);
        if !self.session.shutdown().await {
            anyhow::bail!("Background work timed out at shutdown");
        }
        result
    }

    async fn run_session(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<()> {
        let size = terminal.size()?;
        self.session.state.terminal_width = size.width;
        self.session.state.terminal_height = size.height;
        self.session.initialize().await?;
        let crate::app::runtime::Session {
            state,
            audio,
            config,
            events: event_tx,
            completions: event_rx,
            clock: playback_clock,
            ..
        } = &mut self.session;
        let tick_rate = Duration::from_millis(100);
        let idle_render_rate = Duration::from_millis(33);
        let mut last_tick = Instant::now();
        let mut last_render = Instant::now()
            .checked_sub(idle_render_rate)
            .unwrap_or_else(Instant::now);
        let mut dirty = true;

        self.shutdown.store(false, Ordering::Release);
        let mut input_reader = InputReader::spawn(event_tx.clone(), self.shutdown.clone())?;

        crate::ui::theme::set_theme(state.theme);

        let mut scene_motion =
            ui::motion::SceneMotion::new(std::env::var("TERM").as_deref() != Ok("dumb"));

        if std::env::var("TERM").as_deref() != Ok("dumb") {
            let player = state.tui_player_raised();
            state.tui_motion.initialize(
                state.tui_sidebar_visible,
                state.tui_queue_visible,
                player,
                Instant::now(),
            );
        }
        let mut picker = ui::playlist_picker::PlaylistPicker::default();
        // Main loop
        loop {
            picker.sync(state);
            let now = Instant::now();
            let player = state.tui_player_raised();
            state.tui_motion.sync(
                state.tui_sidebar_visible,
                state.tui_queue_visible,
                player,
                now,
            );
            dirty |= state.tui_motion.active(now) || scene_motion.active(now);
            let render_rate = if state.tui_motion.active(now) || scene_motion.active(now) {
                Duration::from_millis(16)
            } else {
                idle_render_rate
            };

            // Coalesce bursts of input/network completions and cap terminal
            // writes at ~30 FPS. The app tick marks animated state dirty at
            // 10 FPS; idle screens perform no redraws at all.
            if dirty && last_render.elapsed() >= render_rate {
                let mut feedback = None;
                std::io::stdout().sync_update(|_| {
                    terminal
                        .draw(|f| {
                            feedback = Some(ui::render_with_motion(
                                f,
                                state,
                                &mut scene_motion,
                                ui::OutputHost::Terminal,
                            ));
                            picker.draw(f, state);
                        })
                        .map(|_| ())
                })??;
                if let Some(feedback) = feedback {
                    state.apply_render_feedback(feedback);
                }
                dirty =
                    state.tui_motion.active(Instant::now()) || scene_motion.active(Instant::now());
                last_render = now;
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
                Some(event) = event_rx.recv() => {
                    direct_events.push(event);
                }
                _ = tokio::time::sleep(timeout) => {
                    if last_tick.elapsed() >= tick_rate {
                        direct_events.extend(playback_clock.poll(state, audio));

                        last_tick = Instant::now();
                    }
                }
                _ = self.signals.recv() => {
                    signal_quit = true;
                }
            }

            if signal_quit {
                crate::app::dispatch::dispatch_action(
                    crate::app::action::SystemAction::Quit.into(),
                    state,
                    audio,
                    config,
                    event_tx,
                )
                .await?;
                dirty = true;
            }

            for event in direct_events {
                if let Event::InputError(error) | Event::WorkerFailed(error) = &event {
                    anyhow::bail!("{error}");
                }
                let event_needs_render =
                    !matches!(&event, Event::Tick) || tick_requires_render(state);
                let actions = picker
                    .input(&event, state)
                    .unwrap_or_else(|| crate::app::runtime::reduce(event, state, config, event_tx));
                for action in actions {
                    crate::app::dispatch::dispatch_action(action, state, audio, config, event_tx)
                        .await?;
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
}
