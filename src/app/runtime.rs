//! Shared frontend-independent startup and playback maintenance.
use crate::{
    app::{AppState, Event, PlayStatus, event::PlaybackEvent},
    audio::AudioPlayer,
    config::Config,
};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Backend choice supplied by a launch adapter or an isolated test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioMode {
    Native,
    Silent,
}

/// Exclusive owner of a live application session, independent of its presentation.
pub struct Session {
    pub state: AppState,
    pub config: Config,
    pub audio: AudioPlayer,
    pub events: mpsc::Sender<Event>,
    pub completions: mpsc::Receiver<Event>,
    pub clock: PlaybackClock,
    tasks: crate::app::tasks::TaskSession,
    startup: Startup,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Startup {
    Pending,
    Ready,
}
impl Session {
    /// Prepare a session without opening a window or entering terminal raw mode.
    /// # Errors
    /// Returns an error if the blocking initialization worker cannot finish.
    pub async fn open(config: Config, mode: AudioMode) -> anyhow::Result<Self> {
        let audio = match mode {
            AudioMode::Silent => AudioPlayer::new_without_audio(),
            AudioMode::Native => match tokio::task::spawn_blocking(AudioPlayer::new).await? {
                Ok(audio) => audio,
                Err(error) => {
                    tracing::warn!(%error, "Audio device unavailable");
                    AudioPlayer::new_without_audio()
                }
            },
        };
        Ok(Self::with_audio(config, audio))
    }
    /// Construct an unstarted session with an explicitly owned backend.
    pub fn with_audio(config: Config, mut audio: AudioPlayer) -> Self {
        let mut state = AppState::new();
        state.audio_available = audio.has_audio();
        state.gui_multiple_windows = config.ui.desktop_separate_player;
        state.playback.volume = config.playback.default_volume;
        state.transcode_kbps = config.playback.transcode_kbps;
        audio.set_volume(state.playback.volume);
        let (events, completions) = mpsc::channel(256);
        let tasks = crate::app::tasks::TaskSession::new(events.clone());
        Self {
            tasks,
            state,
            config,
            audio,
            events,
            completions,
            clock: PlaybackClock::default(),
            startup: Startup::Pending,
        }
    }
    /// Enter the session's task registry without creating another cancelling owner.
    pub fn task_context(&self) -> crate::app::tasks::TaskContext {
        self.tasks.context()
    }
    /// Start the configured source once; a presentation change does not restart it.
    /// # Errors
    /// Propagates startup dispatch and source initialization errors.
    pub async fn initialize(&mut self) -> anyhow::Result<()> {
        if self.startup == Startup::Ready {
            return Ok(());
        }
        initialize(
            &mut self.state,
            &mut self.audio,
            &mut self.config,
            &self.events,
        )
        .await?;
        self.startup = Startup::Ready;
        Ok(())
    }
    /// Dispatch through the source-aware shared reducers.
    /// # Errors
    /// Propagates dispatch errors without replacing the live state.
    pub async fn dispatch(&mut self, action: crate::app::Action) -> anyhow::Result<()> {
        crate::app::dispatch::dispatch_action(
            action,
            &mut self.state,
            &mut self.audio,
            &mut self.config,
            &self.events,
        )
        .await
    }
    /// Poll the sole backend clock without assigning ownership to a renderer.
    pub fn poll(&mut self) -> Vec<Event> {
        self.clock.poll(&mut self.state, &mut self.audio)
    }
    /// Stop the backend and wait for owned filesystem work to finish.
    pub async fn shutdown(&mut self) -> bool {
        self.tasks.close();
        self.audio.stop();
        crate::app::tasks::finish_blocking(Duration::from_secs(3)).await
    }
}

/// Reduce normalized input and completions against the same application state.
pub fn reduce(
    event: Event,
    state: &mut AppState,
    config: &Config,
    events: &mpsc::Sender<Event>,
) -> Vec<crate::app::Action> {
    let actions = match event {
        Event::Key(key) => {
            state.cache_mgmt.last_input_time = Instant::now();
            crate::app::handlers::key_input::handle_key(key, state, config)
        }
        Event::Paste(text) => super::terminal_text::insert(&text, state, config),
        Event::Resize(width, height) => {
            state.terminal_width = width;
            state.terminal_height = height;
            state.seek_drag = None;
            state.volume_drag = false;
            vec![]
        }
        other => crate::app::dispatch::handle_core_event(other, state, events),
    };
    super::lyrics::reconcile(state, events);
    let mut actions = actions;
    actions.extend(super::player_sync::poll(state, config));
    actions
}

/// Initialize shared saved configuration and the selected source.
/// # Errors
/// Propagates source initialization and dispatch errors.
pub async fn initialize(
    state: &mut AppState,
    audio: &mut AudioPlayer,
    config: &mut Config,
    event_tx: &mpsc::Sender<Event>,
) -> anyhow::Result<()> {
    let root = state.downloads.store.root.clone();
    let downloads =
        crate::app::tasks::spawn_blocking(move || crate::services::downloads::Store::load(root));
    // Both frontends consume the same live sample tap across decoder installs.
    state.vectorscope_tap = audio.sample_tap();

    state
        .last_playlist_destinations
        .clone_from(&config.ui.last_playlist_destinations);
    state.palette.history = config.ui.command_history.clone();
    if state.palette.history.is_empty() {
        let entries = super::command_palette::global_entries();
        for label in config.ui.desktop_command_history.iter().rev() {
            if let Some(entry) = entries.iter().find(|entry| {
                entry.label == *label
                    || super::command_catalog::label(&entry.command, &entry.label) == *label
            }) && let Some(key) = super::command_history::key(&entry.command)
            {
                state.palette.history.remember(key);
            }
        }
    }

    // Apply the configured theme before the first frame.
    state.theme = crate::app::theme::ThemeName::from_config(&config.ui.theme);

    // Restore artwork mode preference
    state.artwork.mode = crate::app::state::ArtworkMode::from_config(&config.ui.artwork_mode);

    // Mirror the per-service "external search enabled" toggles
    // from config onto AppState so the palette / context menus /
    // menu bar can gate their entries without threading Config
    // through every render path. dispatch_settings updates these
    // when the user flips a Settings toggle.
    state.external_search.apple_music = config.ui.enable_apple_music_search;
    state.web_search_provider = config.ui.web_search_provider;
    state.external_search.spotify = config.ui.enable_spotify_search;
    state.external_search.youtube = config.ui.enable_youtube_search;

    state.hidden_sections = config.ui.hidden_sections.clone();
    state.desktop_hidden_library_roots = config.ui.desktop_hidden_library_roots.clone();
    state.remix_allow_same_artist = config.ui.remix_allow_same_artist;
    state.show_compilation_only_artists = config.ui.show_compilation_only_artists;
    state
        .player_sync
        .history
        .clone_from(&config.player_sync_history);
    state.album_grid = config.ui.album_layout == crate::config::AlbumLayout::Grid;
    state.singles_track_limit = config
        .ui
        .singles_track_limit
        .clamp(1, crate::services::singles::MAX_LIMIT);
    state.desktop_collapsed_sections = config.ui.desktop_collapsed_sections.clone();
    state.hidden_collections = config.ui.hidden_collections.clone();

    // Mirror saved per-playlist view toggles (group-by-album,
    // show-artwork) so the playlist-tracks-column event handlers
    // can apply them without a config handle.
    state.playlist_views.clear();
    for (lib, settings) in &config.ui.library_view_settings {
        state
            .playlist_views
            .insert(lib.clone(), settings.playlists.clone());
    }

    // Restore normal startup; directory discovery is independent of the
    // active library and never opens an overlay or starts playback.
    crate::app::sources::manager::initialize(state, config);

    let source = if let Some(choice) = crate::app::sources::manager::startup_choice(config) {
        crate::app::dispatch::dispatch_action(choice.action(), state, audio, config, event_tx).await
    } else {
        state.view = crate::app::state::View::Browse;
        state.set_status("Add a library in Settings".into());
        Ok(())
    };
    // The selected source can begin loading independently; adapters only drain its
    // completions after the local index is ready.
    match downloads.await? {
        Ok(store) => state.downloads.store = store,
        Err(error) => {
            state.downloads.storage_error = Some(error.to_string());
            state.set_error(format!("Downloads: {error}"));
        }
    }
    source
}

/// One natural-end event per playback attempt, shared by both frontends.
#[derive(Default)]
pub struct PlaybackClock {
    attempt: Option<PlaybackAttempt>,
}
struct PlaybackAttempt {
    playback_id: u64,
    started: Option<Instant>,
    phase: PlaybackPhase,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum PlaybackPhase {
    Monitoring,
    Mixing,
    Ended,
}
impl PlaybackClock {
    pub fn poll(&mut self, state: &mut AppState, audio: &mut AudioPlayer) -> Vec<Event> {
        let mut events = Vec::new();
        let next = state.next_track().map(|track| track.rating_key.clone());
        if next.as_deref() != state.sources.next_waveform.key() {
            state.sources.nav_tasks.remove("next-waveform");
            state.sources.next_waveform = crate::app::sources::NextWaveform::Idle;
        }
        if (state.playback_waveform_available || state.visualizers_available)
            && (!state.downloads.is_offline()
                || state
                    .next_track()
                    .is_some_and(|track| state.downloads.store.find(track).is_some()))
            && state.sources.active.navidrome().is_some()
            && matches!(
                state.playback.status,
                PlayStatus::Playing | PlayStatus::Paused
            )
            && audio.download_complete()
            && next.is_some()
            && next.as_deref() != state.sources.next_waveform.key()
        {
            events.push(Event::Effect(
                crate::app::action::SystemAction::PrefetchNextWaveform.into(),
            ));
        }
        if state.playback.status == PlayStatus::Playing {
            let attempt = self.attempt.get_or_insert(PlaybackAttempt {
                playback_id: state.playback.request_id,
                started: state.playback.playback_started_at,
                phase: PlaybackPhase::Monitoring,
            });
            if attempt.playback_id != state.playback.request_id
                || attempt.started != state.playback.playback_started_at
            {
                *attempt = PlaybackAttempt {
                    playback_id: state.playback.request_id,
                    started: state.playback.playback_started_at,
                    phase: PlaybackPhase::Monitoring,
                };
            }
            if let Some(position) = audio.position() {
                state.playback.position_ms = position.as_millis() as u64;
            }
            if state.consecutive_playback_errors > 0
                && attempt
                    .started
                    .is_some_and(|t| t.elapsed() >= Duration::from_secs(5))
            {
                state.consecutive_playback_errors = 0;
            }
            if attempt.phase == PlaybackPhase::Monitoring
                && state.playback.auto_mix
                && !audio.is_seeking()
                && state.seek_drag.is_none()
                && state.playback.duration_ms >= 20_000
                && state.playback.position_ms >= state.playback.duration_ms.saturating_sub(4_000)
                && state.playback.position_ms < state.playback.duration_ms
                && state.next_track().is_some()
                && state.queue.repeat != crate::app::state::RepeatMode::Track
            {
                attempt.phase = PlaybackPhase::Mixing;
                events.push(Event::Effect(
                    crate::app::action::PlaybackAction::MixNext {
                        playback_id: state.playback.request_id,
                    }
                    .into(),
                ));
            }
            if attempt.phase != PlaybackPhase::Ended
                && attempt
                    .started
                    .is_some_and(|t| t.elapsed() >= Duration::from_secs(1))
                && audio.is_finished()
            {
                attempt.phase = PlaybackPhase::Ended;
                let pos = audio
                    .position()
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(state.playback.position_ms);
                let duration = state.playback.duration_ms;
                events.push(
                    if duration == 0
                        || pos >= duration * 90 / 100
                        || (duration > 5000 && pos >= duration.saturating_sub(5000))
                    {
                        PlaybackEvent::TrackEnded.into()
                    } else {
                        PlaybackEvent::PlaybackError {
                            playback_id: Some(state.playback.request_id),
                            message: "Track ended prematurely".into(),
                        }
                        .into()
                    },
                );
            }
        }
        events.extend(audio.take_failures().into_iter().map(Event::from));
        events.push(Event::Tick);
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_waveform_demand_is_independent_of_visualizers() {
        use crate::app::{Action, action::SystemAction, state::View};
        let mut state = AppState::new();
        state.visualizers_available = false;
        state.playback_waveform_available = true;
        state.queue.tracks.push(crate::library::models::Track {
            rating_key: "song".into(),
            ..Default::default()
        });
        state.queue.index = Some(0);
        let (tx, _) = tokio::sync::mpsc::channel(32);
        let hidden = crate::app::dispatch::handle_core_event(Event::Tick, &mut state, &tx);
        assert!(
            !hidden
                .iter()
                .any(|action| matches!(action, Action::System(SystemAction::LoadWaveform)))
        );
        state.view = View::NowPlaying;
        let visible = crate::app::dispatch::handle_core_event(Event::Tick, &mut state, &tx);
        assert!(
            visible
                .iter()
                .any(|action| matches!(action, Action::System(SystemAction::LoadWaveform)))
        );
        assert!(
            !visible
                .iter()
                .any(|action| matches!(action, Action::System(SystemAction::LoadSpectrogram)))
        );
    }

    #[tokio::test]
    async fn queue_changes_cancel_speculative_work_without_changing_audio_identity() {
        let mut state = AppState::new();
        state.queue.tracks = ["current", "next", "replacement"]
            .map(|id| crate::library::models::Track {
                rating_key: id.into(),
                ..Default::default()
            })
            .into();
        state.queue.index = Some(1);
        state.playback.status = PlayStatus::Playing;
        state.playback.request_id = 42;
        let task = tokio::spawn(std::future::pending::<()>());
        state.sources.next_waveform = crate::app::sources::NextWaveform::Loading("next".into());
        state.sources.nav_tasks.insert(
            "next-waveform",
            (1, crate::app::tasks::TaskLease::new(&task)),
        );
        let mut audio = AudioPlayer::new_without_audio();
        PlaybackClock::default().poll(&mut state, &mut audio);
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!state.sources.nav_tasks.contains_key("next-waveform"));
        assert!(matches!(
            state.sources.next_waveform,
            crate::app::sources::NextWaveform::Idle
        ));
        assert_eq!(state.playback.request_id, 42);
    }

    #[tokio::test]
    async fn host_without_visualizers_never_starts_analysis_for_now_playing() {
        use crate::app::{Action, action::SystemAction, state::View};
        let mut session = Session::with_audio(Config::default(), AudioPlayer::new_without_audio());
        session.state.visualizers_available = false;
        session.state.view = View::NowPlaying;
        session
            .state
            .queue
            .tracks
            .push(crate::library::models::Track {
                rating_key: "song".into(),
                ..Default::default()
            });
        session.state.queue.index = Some(0);
        let actions = crate::app::dispatch::handle_core_event(
            Event::Tick,
            &mut session.state,
            &session.events,
        );
        assert!(!actions.iter().any(|action| matches!(
            action,
            Action::System(SystemAction::LoadWaveform | SystemAction::LoadSpectrogram)
        )));
        session
            .dispatch(SystemAction::LoadWaveform.into())
            .await
            .unwrap();
        session
            .dispatch(SystemAction::LoadSpectrogram.into())
            .await
            .unwrap();
        assert!(!session.state.waveform.generating());
        assert!(!session.state.spectrogram.generating());
        assert!(session.state.waveform.track_key.is_none());
    }

    #[tokio::test]
    async fn live_session_initializes_once_and_keeps_playback_and_navigation() {
        let mut session = Session::with_audio(Config::default(), AudioPlayer::new_without_audio());
        let tasks = session.task_context();
        tasks
            .run(async {
                session.initialize().await.unwrap();
                session.state.playback.position_ms = 12_345;
                session.state.playback.status = PlayStatus::Paused;
                session.state.artist_nav.reset("Retained", vec![]);
                session.config.playback.default_volume = 0.2;
                session.initialize().await.unwrap();
                assert_eq!(session.state.playback.position_ms, 12_345);
                assert_eq!(session.state.playback.status, PlayStatus::Paused);
                assert_eq!(session.state.artist_nav.columns[0].title, "Retained");
                assert_ne!(session.state.playback.volume, 0.2);
            })
            .await;
        drop(tasks);
        assert!(session.shutdown().await);
    }

    #[tokio::test]
    async fn startup_connects_backend_pcm_to_the_shared_tick_consumers() {
        let mut state = AppState::new();
        let mut audio = AudioPlayer::with_test_samples(&[(0.5, 0.25), (-0.5, -0.25)]);
        let (tx, _rx) = mpsc::channel(16);
        initialize(&mut state, &mut audio, &mut Config::default(), &tx)
            .await
            .unwrap();
        assert!(state.vectorscope_tap.is_some());
        state.playback.status = PlayStatus::Playing;
        for event in PlaybackClock::default().poll(&mut state, &mut audio) {
            crate::app::dispatch::handle_core_event(event, &mut state, &tx);
        }
        assert_eq!(state.vectorscope_buffer.len(), 2);
        assert!(state.studio_meters.available);
        assert!((state.studio_meters.rms[0] - 0.5).abs() < 0.0001);
        assert!((state.studio_meters.rms[1] - 0.25).abs() < 0.0001);
    }

    #[test]
    fn idle_clock_only_emits_maintenance() {
        let mut state = AppState::new();
        let mut audio = AudioPlayer::new_without_audio();
        let events = PlaybackClock::default().poll(&mut state, &mut audio);
        assert!(matches!(events.as_slice(), [Event::Tick]));
    }

    #[test]
    fn cold_playback_does_not_report_end() {
        let mut state = AppState::new();
        state.playback.status = PlayStatus::Playing;
        state.playback.playback_started_at = Some(Instant::now());
        let mut audio = AudioPlayer::new_without_audio();
        let events = PlaybackClock::default().poll(&mut state, &mut audio);
        assert!(!events.iter().any(|event| matches!(
            event,
            Event::Playback(PlaybackEvent::TrackEnded | PlaybackEvent::PlaybackError { .. })
        )));
    }
    #[test]
    fn playback_end_is_once_per_attempt_even_when_start_times_are_equal() {
        let mut state = AppState::new();
        state.playback.status = PlayStatus::Playing;
        state.playback.playback_started_at = Some(Instant::now() - Duration::from_secs(2));
        state.playback.duration_ms = 10_000;
        state.playback.position_ms = 10_000;
        state.playback.request_id = 1;
        let mut audio = AudioPlayer::with_test_finished(1, Duration::from_secs(10));
        let mut clock = PlaybackClock::default();
        let ended = |events: Vec<Event>| {
            events
                .into_iter()
                .filter(|event| matches!(event, Event::Playback(PlaybackEvent::TrackEnded)))
                .count()
        };
        assert_eq!(ended(clock.poll(&mut state, &mut audio)), 1);
        assert_eq!(ended(clock.poll(&mut state, &mut audio)), 0);
        state.playback.status = PlayStatus::Paused;
        assert_eq!(ended(clock.poll(&mut state, &mut audio)), 0);
        state.playback.status = PlayStatus::Playing;
        assert_eq!(ended(clock.poll(&mut state, &mut audio)), 0);
        state.playback.request_id = 2;
        audio = AudioPlayer::with_test_finished(2, Duration::from_secs(10));
        assert_eq!(ended(clock.poll(&mut state, &mut audio)), 1);
    }
}
