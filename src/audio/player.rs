//! High-level audio player.
//!
//! Network fetching runs on Tokio tasks, decoder/device work lives on one
//! dedicated OS thread, and all completions carry a playback generation. The
//! UI thread only sends commands and reads atomics.

use super::rodio_backend::{
    BufferedInput, PipelineEvent, SampleTap, StreamFailure, StreamingInput,
};
use super::{AudioBackend, RodioBackend};
use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::{Notify, mpsc, watch};

const MAX_AUDIO_BYTES: usize = 512 * 1024 * 1024;
const MAX_RETRIES: usize = 3;
const BACKEND_COMMAND_CAPACITY: usize = 32;
const MAX_PENDING_FAILURES: usize = 64;
const BACKEND_STOPPED: u8 = 0;
const BACKEND_PLAYING: u8 = 1;
const BACKEND_PAUSED: u8 = 2;

#[derive(Debug, Clone)]
pub enum AudioEvent {
    BufferingReady { playback_id: u64 },
    Error { playback_id: u64, message: String },
    SeekFailed { playback_id: u64, message: String },
}

#[derive(Clone, Copy)]
enum StartPolicy {
    Immediate,
    Overlap,
}

#[derive(Clone, Copy)]
enum StreamPurpose {
    Playback,
    Seek,
}

enum BackendCommand {
    PlayData {
        playback_id: u64,
        data: BufferedInput,
        position: Duration,
        events: Option<mpsc::Sender<AudioEvent>>,
    },
    PlayStream {
        playback_id: u64,
        input: StreamingInput,
        events: mpsc::Sender<AudioEvent>,
    },
    Stop,
}

fn record_backend_failure(failures: &Mutex<VecDeque<AudioEvent>>, failure: AudioEvent) {
    let mut failures = super::lock_or_recover(failures);
    if failures.len() == MAX_PENDING_FAILURES {
        failures.pop_front();
    }
    failures.push_back(failure);
}

#[derive(Debug)]
struct SeekRequest {
    playback_id: u64,
    position: Duration,
    input: Option<BufferedInput>,
    stop_stream: Option<watch::Sender<bool>>,
    events: Option<mpsc::Sender<AudioEvent>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActivePlayback {
    Streaming(u64),
    Indexed(u64),
}
impl ActivePlayback {
    fn id(self) -> u64 {
        match self {
            Self::Streaming(id) | Self::Indexed(id) => id,
        }
    }
}

struct BackendState {
    transition_ms: AtomicU64,
    playback_id: AtomicU64,
    pending_seek: Mutex<Option<SeekRequest>>,
    mode: AtomicU8,
    position_ms: AtomicU64,
    seek_target_ms: AtomicU64,
    finished: AtomicBool,
    volume_bits: AtomicU32,
    desired_paused: AtomicBool,
    shutdown: AtomicBool,
}

impl BackendState {
    fn new() -> Self {
        Self {
            transition_ms: AtomicU64::new(0),
            playback_id: AtomicU64::new(0),
            pending_seek: Mutex::new(None),
            mode: AtomicU8::new(BACKEND_STOPPED),
            position_ms: AtomicU64::new(0),
            seek_target_ms: AtomicU64::new(u64::MAX),
            finished: AtomicBool::new(false),
            volume_bits: AtomicU32::new(0.8_f32.to_bits()),
            desired_paused: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
        }
    }
}

struct BackendActor {
    commands: std::sync::mpsc::SyncSender<BackendCommand>,
    state: Arc<BackendState>,
    failures: Arc<Mutex<VecDeque<AudioEvent>>>,
    sample_tap: SampleTap,
    thread: Option<JoinHandle<()>>,
}

impl BackendActor {
    fn spawn(generation: Arc<AtomicU64>, timeout: Duration) -> Result<Self> {
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel(BACKEND_COMMAND_CAPACITY);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let state = Arc::new(BackendState::new());
        let failures = Arc::new(Mutex::new(VecDeque::new()));
        let actor_state = state.clone();
        let actor_failures = failures.clone();

        let thread = std::thread::Builder::new()
            .name("textamp-audio-actor".to_string())
            .spawn(move || {
                let backend = RodioBackend::new();
                let mut backend = match backend {
                    Ok(backend) => backend,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };

                let sample_tap = backend.sample_tap();
                if ready_tx.send(Ok(sample_tap)).is_err() {
                    // The caller timed out; do not leave a detached audio actor.
                    return;
                }

                let mut last_underruns = 0;
                let mut stream_events: Option<(u64, mpsc::Sender<AudioEvent>)> = None;
                let mut active_playback: Option<ActivePlayback> = None;
                let mut applied_volume = backend.volume();
                loop {
                    if actor_state.shutdown.load(Ordering::Acquire) {
                        backend.stop();
                        break;
                    }
                    // Generation invalidation is the out-of-band high-priority
                    // stop path. It remains reliable even if the bounded
                    // command mailbox is temporarily full.
                    if active_playback
                        .is_some_and(|playback| generation.load(Ordering::Acquire) != playback.id())
                    {
                        let transition = actor_state.transition_ms.swap(0, Ordering::AcqRel);
                        if transition > 0 {
                            backend.begin_transition(Duration::from_millis(transition));
                        } else {
                            backend.stop();
                        }
                        active_playback = None;
                        stream_events = None;
                        actor_state.mode.store(BACKEND_STOPPED, Ordering::Release);
                        actor_state.position_ms.store(0, Ordering::Release);
                        actor_state.finished.store(false, Ordering::Release);
                    }

                    // Pause/resume is level-triggered rather than queued, so
                    // rapid input or a full playback-command mailbox cannot
                    // lose the user's final transport state.
                    if active_playback.is_some() {
                        let desired_paused = actor_state.desired_paused.load(Ordering::Acquire);
                        let mode = actor_state.mode.load(Ordering::Acquire);
                        if desired_paused && mode != BACKEND_PAUSED {
                            backend.pause();
                            actor_state.mode.store(BACKEND_PAUSED, Ordering::Release);
                        } else if !desired_paused && mode == BACKEND_PAUSED {
                            backend.resume();
                            actor_state.mode.store(BACKEND_PLAYING, Ordering::Release);
                        }
                    }

                    match command_rx.recv_timeout(Duration::from_millis(20)) {
                        Ok(BackendCommand::PlayData {
                            playback_id,
                            data,
                            position,
                            events,
                        }) => {
                            if generation.load(Ordering::Acquire) != playback_id {
                                continue;
                            }
                            let transition = actor_state.transition_ms.swap(0, Ordering::AcqRel);
                            if transition > 0 && active_playback.is_some() {
                                backend.begin_transition(Duration::from_millis(transition));
                            }
                            actor_state.finished.store(false, Ordering::Release);
                            actor_state.position_ms.store(0, Ordering::Release);
                            actor_state
                                .playback_id
                                .store(playback_id, Ordering::Release);
                            stream_events = None;
                            match backend.start_buffered(data, position) {
                                Ok(()) if generation.load(Ordering::Acquire) == playback_id => {
                                    actor_state.mode.store(BACKEND_PLAYING, Ordering::Release);
                                    active_playback = Some(ActivePlayback::Indexed(playback_id));
                                    if actor_state.desired_paused.load(Ordering::Acquire) {
                                        backend.pause();
                                        actor_state.mode.store(BACKEND_PAUSED, Ordering::Release);
                                    }
                                    if let Some(events) = events {
                                        let _ = events
                                            .try_send(AudioEvent::BufferingReady { playback_id });
                                    }
                                }
                                Ok(()) => backend.stop(),
                                Err(error) => {
                                    actor_state.mode.store(BACKEND_STOPPED, Ordering::Release);
                                    if generation.load(Ordering::Acquire) == playback_id {
                                        record_backend_failure(
                                            &actor_failures,
                                            AudioEvent::Error {
                                                playback_id,
                                                message: error.to_string(),
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        Ok(BackendCommand::PlayStream {
                            playback_id,
                            input,
                            events,
                        }) => {
                            if generation.load(Ordering::Acquire) != playback_id
                                || active_playback == Some(ActivePlayback::Indexed(playback_id))
                            {
                                continue;
                            }
                            let transition = actor_state.transition_ms.swap(0, Ordering::AcqRel);
                            if transition > 0 && active_playback.is_some() {
                                backend.begin_transition(Duration::from_millis(transition));
                            }
                            actor_state.finished.store(false, Ordering::Release);
                            actor_state.position_ms.store(0, Ordering::Release);
                            actor_state.mode.store(BACKEND_STOPPED, Ordering::Release);
                            actor_state
                                .playback_id
                                .store(playback_id, Ordering::Release);
                            match backend.start_stream(playback_id, input) {
                                Ok(()) => {
                                    active_playback = Some(ActivePlayback::Streaming(playback_id));
                                    stream_events = Some((playback_id, events));
                                }
                                Err(error) => {
                                    stream_events = None;
                                    record_backend_failure(
                                        &actor_failures,
                                        AudioEvent::Error {
                                            playback_id,
                                            message: error.to_string(),
                                        },
                                    );
                                }
                            }
                        }
                        Ok(BackendCommand::Stop) => {
                            backend.stop();
                            active_playback = None;
                            stream_events = None;
                            actor_state.mode.store(BACKEND_STOPPED, Ordering::Release);
                            actor_state.position_ms.store(0, Ordering::Release);
                            actor_state.finished.store(false, Ordering::Release);
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            backend.stop();
                            break;
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    }

                    // A streamed request waits for seekable input; the current
                    // pipeline keeps running while its replacement is prepared.
                    let seek = {
                        let mut pending = super::lock_or_recover(&actor_state.pending_seek);
                        if pending.as_ref().is_some_and(|seek| {
                            seek.input.is_some()
                                || active_playback
                                    == Some(ActivePlayback::Indexed(seek.playback_id))
                        }) {
                            actor_state.finished.store(false, Ordering::Release);
                            pending.take()
                        } else {
                            None
                        }
                    };
                    if let Some(SeekRequest {
                        playback_id,
                        position,
                        input,
                        stop_stream,
                        events,
                    }) = seek
                        && generation.load(Ordering::Acquire) == playback_id
                    {
                        let succeeded = if let Some(input) = input {
                            backend.seek_buffered(input, position).is_ok()
                        } else {
                            backend.seek(position)
                        };
                        let _ = actor_state.seek_target_ms.compare_exchange(
                            position.as_millis() as u64,
                            u64::MAX,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        );
                        if succeeded {
                            if let Some(stop_stream) = stop_stream {
                                stop_stream.send_replace(true);
                            }
                            if let Some(events) = events {
                                let _ = events.try_send(AudioEvent::BufferingReady { playback_id });
                            }
                            active_playback = Some(ActivePlayback::Indexed(playback_id));
                            actor_state
                                .playback_id
                                .store(playback_id, Ordering::Release);
                            actor_state.finished.store(false, Ordering::Release);
                            if actor_state.desired_paused.load(Ordering::Acquire) {
                                backend.pause();
                            } else {
                                backend.resume();
                            }
                        } else {
                            record_backend_failure(
                                &actor_failures,
                                AudioEvent::SeekFailed {
                                    playback_id,
                                    message: "Cannot seek in this track".into(),
                                },
                            );
                        }
                    }

                    for event in
                        backend.poll_pipeline(actor_state.desired_paused.load(Ordering::Acquire))
                    {
                        match event {
                            PipelineEvent::Ready { playback_id }
                                if generation.load(Ordering::Acquire) == playback_id =>
                            {
                                actor_state.mode.store(BACKEND_PLAYING, Ordering::Release);
                                if let Some((event_id, events)) = &stream_events
                                    && *event_id == playback_id
                                {
                                    let _ =
                                        events.try_send(AudioEvent::BufferingReady { playback_id });
                                }
                            }
                            PipelineEvent::Failed {
                                playback_id,
                                message,
                            } if generation.load(Ordering::Acquire) == playback_id => {
                                actor_state.mode.store(BACKEND_STOPPED, Ordering::Release);
                                active_playback = None;
                                stream_events = None;
                                record_backend_failure(
                                    &actor_failures,
                                    AudioEvent::Error {
                                        playback_id,
                                        message,
                                    },
                                );
                            }
                            _ => {}
                        }
                    }

                    let requested_volume =
                        f32::from_bits(actor_state.volume_bits.load(Ordering::Acquire));
                    if requested_volume.to_bits() != applied_volume.to_bits() {
                        backend.set_volume(requested_volume);
                        applied_volume = backend.volume();
                        actor_state
                            .volume_bits
                            .store(applied_volume.to_bits(), Ordering::Release);
                    }

                    if let Some(position) = backend.position() {
                        actor_state
                            .position_ms
                            .store(position.as_millis() as u64, Ordering::Release);
                    }
                    if backend.is_finished() {
                        actor_state.finished.store(true, Ordering::Release);
                        actor_state.mode.store(BACKEND_STOPPED, Ordering::Release);
                    } else if backend.is_paused() {
                        actor_state.mode.store(BACKEND_PAUSED, Ordering::Release);
                    } else if backend.is_playing() {
                        actor_state.mode.store(BACKEND_PLAYING, Ordering::Release);
                    }

                    let underruns = backend.underrun_count();
                    if underruns.saturating_sub(last_underruns) >= 48_000 {
                        tracing::warn!(
                            "Audio PCM ring underrun: {} silent samples since last report",
                            underruns - last_underruns
                        );
                        last_underruns = underruns;
                    }
                }
            })?;

        let sample_tap = match ready_rx.recv_timeout(timeout) {
            Ok(Ok(sample_tap)) => sample_tap,
            Ok(Err(error)) => {
                let _ = thread.join();
                return Err(anyhow!("Failed to create audio backend: {error}"));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                return Err(anyhow!("Audio initialization timed out"));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                return Err(anyhow!("Audio initialization thread exited"));
            }
        };

        Ok(Self {
            commands: command_tx,
            state,
            failures,
            sample_tap,
            thread: Some(thread),
        })
    }

    fn send(&self, command: BackendCommand) -> Result<()> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => {
                    anyhow!("Audio actor command queue is full")
                }
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    anyhow!("Audio actor is unavailable")
                }
            })
    }
}

impl Drop for BackendActor {
    fn drop(&mut self) {
        self.state.shutdown.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            super::finish_thread(thread, "audio actor", Duration::from_secs(1));
        }
    }
}

/// Retain only the active stream's request, so seeking can reopen it with the
/// same credentials and fallback. Stop drops it and cancels pending work.
enum RemoteSeek {
    Unprepared,
    Preparing,
    Ready(crate::library::MediaFile),
}

struct StreamSession {
    url: String,
    headers: HeaderMap,
    fallback_url: Option<String>,
    events: mpsc::Sender<AudioEvent>,
    client: reqwest::Client,
    seek: Mutex<RemoteSeek>,
    seek_changed: Notify,
    stop_stream: watch::Sender<bool>,
}

/// Immutable access to the current download, fenced by the playback identity.
pub struct AnalysisInput {
    session: Arc<StreamSession>,
    generation: Arc<AtomicU64>,
    cancellation: watch::Receiver<u64>,
    playback_id: u64,
}
impl AnalysisInput {
    pub async fn wait(mut self) -> Result<crate::library::MediaFile, String> {
        loop {
            if !is_current(&self.generation, self.playback_id) {
                return Err("Track changed".into());
            }
            // Register before reading the state so a transition during the read cannot be missed.
            let changed = self.session.seek_changed.notified();
            {
                match &*super::lock_or_recover(&self.session.seek) {
                    RemoteSeek::Ready(file) => return Ok(file.clone()),
                    RemoteSeek::Unprepared => return Err("Audio download did not complete".into()),
                    RemoteSeek::Preparing => {}
                }
            }
            tokio::select! {
                _ = changed => {}
                _ = wait_for_cancellation(&mut self.cancellation, self.playback_id) => {
                    return Err("Track changed".into());
                }
            }
        }
    }
}

pub struct AudioPlayer {
    start_policy: StartPolicy,
    backend: Option<BackendActor>,
    output_session: Option<Box<dyn Fn(bool) -> Result<()> + Send>>,
    volume: f32,
    /// Local input retained by the mobile host for recovery after iOS invalidates its Audio Unit.
    resume_input: Option<BufferedInput>,
    stream: Option<Arc<StreamSession>>,
    playback_generation: Arc<AtomicU64>,
    cancellation_tx: watch::Sender<u64>,
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let mut player = Self::new_without_audio();
        player.try_attach_backend()?;
        Ok(player)
    }

    /// Open the output only when the host's first playback request reaches the player.
    #[cfg(any(feature = "mobile", test))]
    pub(crate) fn with_deferred_output(
        output_session: impl Fn(bool) -> Result<()> + Send + 'static,
    ) -> Self {
        let mut player = Self::new_without_audio();
        player.output_session = Some(Box::new(output_session));
        player
    }

    pub fn new_without_audio() -> Self {
        let (cancellation_tx, _) = watch::channel(0);
        Self {
            backend: None,
            output_session: None,
            volume: 0.8,
            resume_input: None,
            stream: None,

            playback_generation: Arc::new(AtomicU64::new(0)),
            cancellation_tx,
            start_policy: StartPolicy::Immediate,
        }
    }

    fn invalidate_playback(&self) -> u64 {
        let playback_id = self
            .playback_generation
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        self.cancellation_tx.send_replace(playback_id);
        playback_id
    }

    pub fn playback_id(&self) -> u64 {
        self.playback_generation.load(Ordering::Acquire)
    }

    pub fn play_url(
        &mut self,
        url: &str,
        event_tx: mpsc::Sender<AudioEvent>,
        http_client: reqwest::Client,
    ) -> Result<()> {
        self.play_url_with_headers(url, HeaderMap::new(), None, event_tx, http_client)
    }

    pub fn play_url_with_headers(
        &mut self,
        url: &str,
        headers: HeaderMap,
        fallback_url: Option<String>,
        event_tx: mpsc::Sender<AudioEvent>,
        http_client: reqwest::Client,
    ) -> Result<()> {
        self.stop_for_preparation();
        self.start_policy = StartPolicy::Immediate;
        self.open_deferred_output()?;
        let (stop_stream, _) = watch::channel(false);
        self.stream = Some(Arc::new(StreamSession {
            url: url.to_string(),
            headers,
            fallback_url,
            events: event_tx,
            client: http_client,
            seek: Mutex::new(RemoteSeek::Preparing),
            seek_changed: Notify::new(),
            stop_stream,
        }));
        self.start_stream_at(Duration::ZERO, StreamPurpose::Playback)
    }

    fn start_stream_at(&self, position: Duration, purpose: StreamPurpose) -> Result<()> {
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| anyhow!("No audio output device is available"))?;
        let backend_commands = backend.commands.clone();
        let backend_state = backend.state.clone();
        let session = self
            .stream
            .clone()
            .ok_or_else(|| anyhow!("No active stream"))?;
        let playback_id = self.playback_id();
        let generation = self.playback_generation.clone();
        let mut cancellation = self.cancellation_tx.subscribe();
        tokio::spawn(async move {
            // A drag shares one preparation task and keeps only its latest target.
            if matches!(purpose, StreamPurpose::Seek) {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(150)) => {}
                    _ = wait_for_cancellation(&mut cancellation, playback_id) => return,
                }
            }
            let mut stop_stream = session.stop_stream.subscribe();
            let fetch = stream_audio(
                StreamRequest {
                    primary_url: &session.url,
                    fallback_url: session.fallback_url.as_deref(),
                    headers: &session.headers,
                    client: &session.client,
                    start_position: position,
                    purpose,
                    seek: &session.seek,
                    seek_changed: &session.seek_changed,
                    pending_seek: &backend_state.pending_seek,
                },
                playback_id,
                &generation,
                &mut cancellation,
                &backend_commands,
                &session.events,
            );
            let failure = if matches!(purpose, StreamPurpose::Playback) {
                tokio::select! {
                    result = fetch => result,
                    _ = async {
                        while !*stop_stream.borrow_and_update() {
                            if stop_stream.changed().await.is_err() { break; }
                        }
                    } => None
                }
            } else {
                fetch.await
            };
            {
                let mut seek = super::lock_or_recover(&session.seek);
                if matches!(*seek, RemoteSeek::Preparing) {
                    *seek = RemoteSeek::Unprepared;
                    let mut pending = super::lock_or_recover(&backend_state.pending_seek);
                    if pending
                        .as_ref()
                        .is_some_and(|seek| seek.playback_id == playback_id)
                    {
                        *pending = None;
                        backend_state
                            .seek_target_ms
                            .store(u64::MAX, Ordering::Release);
                    }
                    drop(pending);
                    drop(seek);
                    session.seek_changed.notify_waiters();
                }
            }
            if let Some(failure) = failure
                && is_current(&generation, playback_id)
            {
                let _ = session.events.send(failure).await;
            }
        });
        Ok(())
    }

    /// Hardware-free backend fixture for startup/tap integration tests.
    #[cfg(test)]
    pub(crate) fn with_test_samples(samples: &[(f32, f32)]) -> Self {
        let mut player = Self::new_without_audio();
        let (commands, _receiver) = std::sync::mpsc::sync_channel(BACKEND_COMMAND_CAPACITY);
        player.backend = Some(BackendActor {
            commands,
            state: Arc::new(BackendState::new()),
            failures: Arc::default(),
            sample_tap: SampleTap::from_test_samples(samples),
            thread: None,
        });
        player
    }

    /// Finished hardware-free playback attempt for clock integration tests.
    #[cfg(test)]
    pub(crate) fn with_test_finished(playback_id: u64, position: Duration) -> Self {
        let player = Self::with_test_samples(&[]);
        player
            .playback_generation
            .store(playback_id, Ordering::Release);
        if let Some(backend) = &player.backend {
            backend
                .state
                .playback_id
                .store(playback_id, Ordering::Release);
            backend
                .state
                .position_ms
                .store(position.as_millis() as u64, Ordering::Release);
            backend.state.finished.store(true, Ordering::Release);
        }
        player
    }

    pub(crate) fn has_resume_input(&self) -> bool {
        self.resume_input.is_some() || self.stream.is_some()
    }

    pub fn has_audio(&self) -> bool {
        self.backend.is_some()
    }

    /// Share the current compressed download with analysis instead of issuing another request.
    pub fn analysis_input(&self) -> Option<AnalysisInput> {
        Some(AnalysisInput {
            session: self.stream.clone()?,
            generation: self.playback_generation.clone(),
            cancellation: self.cancellation_tx.subscribe(),
            playback_id: self.playback_id(),
        })
    }

    /// Background work must not compete with the current compressed transfer.
    pub fn download_complete(&self) -> bool {
        self.stream.as_ref().is_none_or(|session| {
            matches!(
                &*super::lock_or_recover(&session.seek),
                RemoteSeek::Ready(_)
            )
        })
    }

    pub fn sample_tap(&self) -> Option<SampleTap> {
        self.backend
            .as_ref()
            .map(|backend| backend.sample_tap.clone())
    }

    pub fn try_attach_backend(&mut self) -> Result<bool> {
        if self.backend.is_some() {
            return Ok(false);
        }
        let backend =
            BackendActor::spawn(self.playback_generation.clone(), Duration::from_secs(5))?;
        self.backend = Some(backend);
        self.set_volume(self.volume);
        Ok(true)
    }

    fn open_deferred_output(&mut self) -> Result<()> {
        if self.output_session.is_some() {
            self.set_output_session_active(true)?;
            if let Err(error) = self.try_attach_backend() {
                let _ = self.set_output_session_active(false);
                return Err(error);
            }
        }
        Ok(())
    }

    fn set_output_session_active(&self, active: bool) -> Result<()> {
        if let Some(session) = &self.output_session {
            session(active)?;
        }
        Ok(())
    }

    pub fn play_data(&mut self, data: Arc<Vec<u8>>) -> Result<()> {
        self.play_input(data.into())
    }

    pub fn play_file(&mut self, file: crate::library::MediaFile) -> Result<()> {
        self.play_input(BufferedInput::File(file))
    }

    fn play_input(&mut self, data: BufferedInput) -> Result<()> {
        self.stop_for_preparation();
        self.start_policy = StartPolicy::Immediate;
        self.open_deferred_output()?;
        let playback_id = self.playback_id();
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| anyhow!("No audio output device is available"))?;
        if self.output_session.is_some() {
            self.resume_input = Some(data.clone());
        }
        backend.send(BackendCommand::PlayData {
            playback_id,
            data,
            position: Duration::ZERO,
            events: None,
        })
    }

    pub fn pause(&mut self) {
        if let Some(backend) = &self.backend {
            backend.state.desired_paused.store(true, Ordering::Release);
        }
    }

    /// The system may dispose the Audio Unit even though the mixer and its sink still exist.
    pub(crate) fn interrupt_output(&mut self) {
        if self.output_session.is_some() {
            self.invalidate_playback();
            self.backend.take();
        }
    }

    pub fn resume(&mut self) -> Result<()> {
        self.resume_at(self.position().unwrap_or_default())
    }

    pub(crate) fn resume_at(&mut self, position: Duration) -> Result<()> {
        if let Some(backend) = &self.backend {
            self.set_output_session_active(true)?;
            backend.state.desired_paused.store(false, Ordering::Release);
            return Ok(());
        }
        if self.stream.is_none() && self.resume_input.is_none() {
            anyhow::bail!("There is no paused audio to resume");
        }
        self.open_deferred_output()?;
        let result = self.restart_interrupted_input(position);
        if result.is_err() {
            self.backend.take();
            let _ = self.set_output_session_active(false);
        }
        result
    }

    fn restart_interrupted_input(&mut self, position: Duration) -> Result<()> {
        let (data, events) = if let Some(input) = self.resume_input.clone() {
            (input, None)
        } else {
            let previous = self.stream.as_ref().context("There is no paused stream")?;
            let cached = match &*super::lock_or_recover(&previous.seek) {
                RemoteSeek::Ready(file) => Some(file.clone()),
                _ => None,
            };
            if let Some(file) = cached {
                (BufferedInput::File(file), Some(previous.events.clone()))
            } else {
                self.stream = Some(Arc::new(StreamSession {
                    url: previous.url.clone(),
                    headers: previous.headers.clone(),
                    fallback_url: previous.fallback_url.clone(),
                    events: previous.events.clone(),
                    client: previous.client.clone(),
                    seek: Mutex::new(RemoteSeek::Preparing),
                    seek_changed: Notify::new(),
                    stop_stream: watch::channel(false).0,
                }));
                return self.start_stream_at(position, StreamPurpose::Playback);
            }
        };
        self.backend
            .as_ref()
            .context("No audio output device is available")?
            .send(BackendCommand::PlayData {
                playback_id: self.playback_id(),
                data,
                position,
                events,
            })
    }

    pub fn stop(&mut self) {
        self.resume_input = None;
        self.start_policy = StartPolicy::Immediate;
        self.stream = None;
        self.invalidate_playback();
        if let Some(backend) = &self.backend {
            backend.state.transition_ms.store(0, Ordering::Release);
            backend
                .state
                .seek_target_ms
                .store(u64::MAX, Ordering::Release);
            backend.state.desired_paused.store(false, Ordering::Release);
            *super::lock_or_recover(&backend.state.pending_seek) = None;
            let _ = backend.send(BackendCommand::Stop);
        }
    }

    pub fn prepare_transition(&mut self, duration: Duration) {
        self.start_policy = StartPolicy::Overlap;
        if let Some(backend) = &self.backend {
            backend
                .state
                .transition_ms
                .store(duration.as_millis().min(6_000) as u64, Ordering::Release);
        }
    }

    pub fn stop_for_preparation(&mut self) {
        if matches!(self.start_policy, StartPolicy::Overlap) {
            if let Some(backend) = &self.backend {
                *super::lock_or_recover(&backend.state.pending_seek) = None;
            }
            self.stream = None;
            self.resume_input = None;
            self.invalidate_playback();
        } else {
            self.stop();
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(backend) = &self.backend {
            backend
                .state
                .volume_bits
                .store(self.volume.to_bits(), Ordering::Release);
        }
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn is_finished(&self) -> bool {
        self.backend.as_ref().is_some_and(|backend| {
            let pending = super::lock_or_recover(&backend.state.pending_seek);
            pending
                .as_ref()
                .is_none_or(|seek| seek.playback_id != self.playback_id())
                && backend.state.playback_id.load(Ordering::Acquire) == self.playback_id()
                && backend.state.finished.load(Ordering::Acquire)
        })
    }

    pub fn is_playing(&self) -> bool {
        self.backend
            .as_ref()
            .is_some_and(|backend| backend.state.mode.load(Ordering::Acquire) == BACKEND_PLAYING)
    }

    pub fn is_paused(&self) -> bool {
        self.backend
            .as_ref()
            .is_some_and(|backend| backend.state.mode.load(Ordering::Acquire) == BACKEND_PAUSED)
    }

    pub fn seek(&mut self, position: Duration) -> Result<()> {
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| anyhow!("No audio output device is available"))?;
        backend
            .state
            .seek_target_ms
            .store(position.as_millis() as u64, Ordering::Release);
        let request = |input, stop_stream, events| SeekRequest {
            playback_id: self.playback_id(),
            position,
            input,
            stop_stream,
            events,
        };
        let prepare = if let Some(session) = &self.stream {
            let mut seek = super::lock_or_recover(&session.seek);
            let (input, prepare) = match &*seek {
                RemoteSeek::Ready(file) => (Some(BufferedInput::File(file.clone())), false),
                RemoteSeek::Preparing => (None, false),
                RemoteSeek::Unprepared => {
                    *seek = RemoteSeek::Preparing;
                    (None, true)
                }
            };
            *super::lock_or_recover(&backend.state.pending_seek) = Some(request(
                input,
                Some(session.stop_stream.clone()),
                Some(session.events.clone()),
            ));
            prepare
        } else {
            *super::lock_or_recover(&backend.state.pending_seek) = Some(request(None, None, None));
            false
        };
        if prepare {
            self.start_stream_at(position, StreamPurpose::Seek)?;
        }
        Ok(())
    }

    pub fn is_seeking(&self) -> bool {
        self.backend
            .as_ref()
            .is_some_and(|backend| backend.state.seek_target_ms.load(Ordering::Acquire) != u64::MAX)
    }

    pub fn position(&self) -> Option<Duration> {
        self.backend
            .as_ref()
            .filter(|backend| {
                backend.state.playback_id.load(Ordering::Acquire) == self.playback_id()
                    && backend.state.mode.load(Ordering::Acquire) != BACKEND_STOPPED
            })
            .map(|backend| {
                let target = backend.state.seek_target_ms.load(Ordering::Acquire);
                Duration::from_millis(if target == u64::MAX {
                    backend.state.position_ms.load(Ordering::Acquire)
                } else {
                    target
                })
            })
    }

    /// Drain decoder/device failures without blocking the UI.
    pub fn take_failures(&self) -> Vec<AudioEvent> {
        let Some(backend) = &self.backend else {
            return Vec::new();
        };
        let mut failures = super::lock_or_recover(&backend.failures);
        failures.drain(..).collect()
    }
}

fn is_current(generation: &AtomicU64, playback_id: u64) -> bool {
    generation.load(Ordering::Acquire) == playback_id
}

async fn wait_for_cancellation(cancellation: &mut watch::Receiver<u64>, playback_id: u64) {
    loop {
        if *cancellation.borrow() != playback_id {
            return;
        }
        if cancellation.changed().await.is_err() {
            return;
        }
    }
}

fn retry_delay(response: &reqwest::Response, attempt: usize) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(1_u64 << attempt.min(4)))
        .min(Duration::from_secs(30))
}

fn redact_url(url: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return "<invalid stream URL>".to_string();
    };
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(key, _)| {
            !["X-Plex-Token", "u", "p", "t", "s", "apiKey"]
                .iter()
                .any(|secret| key.eq_ignore_ascii_case(secret))
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    parsed.set_query(None);
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    if !pairs.is_empty() {
        parsed.query_pairs_mut().extend_pairs(pairs);
    }
    parsed.to_string()
}

struct StreamRequest<'a> {
    primary_url: &'a str,
    fallback_url: Option<&'a str>,
    headers: &'a HeaderMap,
    client: &'a reqwest::Client,
    start_position: Duration,
    purpose: StreamPurpose,
    seek: &'a Mutex<RemoteSeek>,
    seek_changed: &'a Notify,
    pending_seek: &'a Mutex<Option<SeekRequest>>,
}

async fn stream_audio(
    request: StreamRequest<'_>,
    playback_id: u64,
    generation: &AtomicU64,
    cancellation: &mut watch::Receiver<u64>,
    backend_commands: &std::sync::mpsc::SyncSender<BackendCommand>,
    events: &mpsc::Sender<AudioEvent>,
) -> Option<AudioEvent> {
    let StreamRequest {
        primary_url,
        fallback_url,
        headers,
        client,
        start_position,
        purpose,
        seek,
        seek_changed,
        pending_seek,
    } = request;
    let mut last_error = "request cancelled".to_string();

    for url in [Some(primary_url), fallback_url].into_iter().flatten() {
        tracing::debug!("Opening audio stream: {}", redact_url(url));
        for attempt in 0..MAX_RETRIES {
            let request_started = std::time::Instant::now();
            if !is_current(generation, playback_id) {
                return None;
            }

            let response = tokio::select! {
                response = tokio::time::timeout(Duration::from_secs(20), client.get(url).headers(headers.clone()).send()) => response,
                _ = wait_for_cancellation(cancellation, playback_id) => return None,
            };

            let response = response
                .map_err(|_| "audio server timed out")
                .and_then(|response| {
                    response.map_err(|error| {
                        if error.is_timeout() {
                            "audio server timed out"
                        } else if error.is_connect() {
                            "could not connect to the audio server"
                        } else {
                            "audio request failed"
                        }
                    })
                });
            let response = match response {
                Ok(response) => response,
                Err(message) => {
                    last_error = message.into();
                    if attempt + 1 == MAX_RETRIES {
                        break;
                    }
                    let delay = Duration::from_secs(1_u64 << attempt);
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = wait_for_cancellation(cancellation, playback_id) => return None,
                    }
                    continue;
                }
            };

            let status = response.status();
            if !status.is_success() {
                let retryable =
                    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS;
                let delay = retry_delay(&response, attempt);
                last_error = format!("audio server returned HTTP {status}");
                tracing::warn!("Audio request returned HTTP {}", status);
                if !retryable || attempt + 1 == MAX_RETRIES {
                    break;
                }
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = wait_for_cancellation(cancellation, playback_id) => return None,
                }
                continue;
            }

            let mime_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.split(';').next().unwrap_or(value).trim().to_string());
            if mime_type
                .as_deref()
                .is_some_and(|content_type| content_type.contains("text/html"))
            {
                last_error = "server returned HTML instead of audio".to_string();
                break;
            }

            if response
                .content_length()
                .is_some_and(|length| length > MAX_AUDIO_BYTES as u64)
            {
                last_error = format!(
                    "audio response is too large ({} MiB limit)",
                    MAX_AUDIO_BYTES / (1024 * 1024)
                );
                break;
            }

            let mut body = response.bytes_stream();
            let first = tokio::select! {
                chunk = tokio::time::timeout(Duration::from_secs(20), body.next()) => match chunk {
                    Ok(chunk) => chunk,
                    Err(_) => {
                        last_error = "audio server timed out before sending audio".into();
                        break;
                    }
                },
                _ = wait_for_cancellation(cancellation, playback_id) => return None,
            };
            let first = match first {
                Some(Ok(chunk)) if !chunk.is_empty() => chunk,
                Some(Ok(_)) | None => {
                    last_error = "audio server returned an empty response".to_string();
                    if attempt + 1 < MAX_RETRIES {
                        continue;
                    }
                    break;
                }
                Some(Err(_)) => {
                    last_error = "audio transfer failed before playback started".to_string();
                    if attempt + 1 < MAX_RETRIES {
                        continue;
                    }
                    break;
                }
            };

            if request_started.elapsed() >= Duration::from_secs(2) {
                tracing::warn!(
                    elapsed_ms = request_started.elapsed().as_millis(),
                    "Audio first packet delayed"
                );
            }

            if looks_like_html(&first) {
                last_error = "server returned HTML instead of audio".to_string();
                break;
            }

            if matches!(purpose, StreamPurpose::Seek) {
                let spool = async {
                    use tokio::io::AsyncWriteExt;
                    let temporary = tempfile::NamedTempFile::new()?;
                    let mut output = tokio::fs::File::from_std(temporary.reopen()?);
                    output.write_all(&first).await?;
                    let mut transferred = first.len();
                    while let Some(chunk) =
                        tokio::time::timeout(Duration::from_secs(20), body.next()).await?
                    {
                        let chunk = chunk.map_err(|error| error.without_url())?;
                        transferred = transferred.saturating_add(chunk.len());
                        anyhow::ensure!(
                            transferred <= MAX_AUDIO_BYTES,
                            "audio exceeds the size limit"
                        );
                        output.write_all(&chunk).await?;
                    }
                    output.flush().await?;
                    Ok::<_, anyhow::Error>(crate::library::MediaFile::temporary(temporary))
                };
                let file = tokio::select! {
                    result = spool => result,
                    _ = wait_for_cancellation(cancellation, playback_id) => return None,
                };
                match file {
                    Ok(file) => {
                        if !is_current(generation, playback_id) {
                            return None;
                        }
                        let mut remote = super::lock_or_recover(seek);
                        *remote = RemoteSeek::Ready(file.clone());
                        let mut pending = super::lock_or_recover(pending_seek);
                        if let Some(request) = pending
                            .as_mut()
                            .filter(|request| request.playback_id == playback_id)
                        {
                            request.input = Some(BufferedInput::File(file));
                        }
                        drop(pending);
                        drop(remote);
                        seek_changed.notify_waiters();
                        return None;
                    }
                    Err(_) => last_error = "audio transfer failed while preparing seek".into(),
                }
                break;
            }

            let indexed = needs_indexed_mp4(mime_type.as_deref(), &first);
            use tokio::io::AsyncWriteExt;
            let (spool, mut writer) = match super::spool::Spool::create() {
                Ok(pair) => pair,
                Err(error) => {
                    last_error = format!("Cannot prepare audio storage: {error}");
                    break;
                }
            };
            if let Err(error) = async {
                writer.write_all(&first).await?;
                writer.flush().await
            }
            .await
            {
                last_error = format!("Cannot store audio: {error}");
                break;
            }
            let mut transferred = first.len();
            spool.published(transferred);
            let failure = StreamFailure::default();
            if !indexed
                && backend_commands
                    .try_send(BackendCommand::PlayStream {
                        playback_id,
                        input: StreamingInput::from_spool(
                            spool.clone(),
                            failure.clone(),
                            mime_type,
                            start_position,
                        ),
                        events: events.clone(),
                    })
                    .is_err()
            {
                last_error = "audio backend is unavailable".into();
                break;
            }
            let transfer = async {
                loop {
                    let next = tokio::time::timeout(Duration::from_secs(20), body.next())
                        .await
                        .map_err(|_| "audio transfer stalled".to_string())?;
                    let Some(chunk) = next else {
                        break;
                    };
                    let chunk = chunk.map_err(|_| "audio transfer was interrupted".to_string())?;
                    transferred = transferred.saturating_add(chunk.len());
                    if transferred > MAX_AUDIO_BYTES {
                        return Err("audio response exceeded the size limit".into());
                    }
                    writer
                        .write_all(&chunk)
                        .await
                        .map_err(|error| format!("Store audio: {error}"))?;
                    writer
                        .flush()
                        .await
                        .map_err(|error| format!("Store audio: {error}"))?;
                    spool.published(transferred);
                }
                Ok::<_, String>(())
            };
            let result = tokio::select! {
                result = transfer => result,
                _ = wait_for_cancellation(cancellation, playback_id) => {
                    spool.finish(Some("Audio request cancelled".into())); return None;
                }
            };
            match result {
                Ok(()) if is_current(generation, playback_id) => {
                    spool.finish(None);
                    let file = spool.file();
                    if indexed
                        && backend_commands
                            .try_send(BackendCommand::PlayData {
                                playback_id,
                                data: BufferedInput::File(file.clone()),
                                position: start_position,
                                events: Some(events.clone()),
                            })
                            .is_err()
                    {
                        last_error = "audio backend is unavailable".into();
                        break;
                    }
                    *super::lock_or_recover(seek) = RemoteSeek::Ready(file.clone());
                    let mut pending = super::lock_or_recover(pending_seek);
                    if let Some(request) = pending
                        .as_mut()
                        .filter(|request| request.playback_id == playback_id)
                    {
                        request.input = Some(BufferedInput::File(file));
                    }
                    drop(pending);
                    seek_changed.notify_waiters();
                }
                Ok(()) => spool.finish(Some("Audio request superseded".into())),
                Err(error) if indexed => {
                    last_error = error;
                    break;
                }
                Err(error) => {
                    failure.set(error.clone());
                    spool.finish(Some(error));
                }
            }
            return None;
        }
        tracing::warn!(
            "Audio endpoint {} failed before streaming; trying fallback if available",
            redact_url(url)
        );
    }

    Some(match purpose {
        StreamPurpose::Playback => AudioEvent::Error {
            playback_id,
            message: format!("Playback failed: {last_error}"),
        },
        StreamPurpose::Seek => AudioEvent::SeekFailed {
            playback_id,
            message: format!("Cannot seek: {last_error}"),
        },
    })
}

/// A sequential MP4 decoder needs the movie index before the first media box.
/// If the prefix cannot establish that order, use the completed seekable file.
fn needs_indexed_mp4(mime_type: Option<&str>, mut prefix: &[u8]) -> bool {
    if prefix.get(4..8) != Some(b"ftyp")
        && !matches!(
            mime_type,
            Some("audio/mp4" | "audio/m4a" | "audio/x-m4a" | "video/mp4")
        )
    {
        return false;
    }
    while let Some(header) = prefix.get(..8) {
        let kind = &header[4..8];
        if kind == b"moov" {
            return false;
        }
        if matches!(kind, b"mdat" | b"moof") {
            break;
        }
        let short_size = u32::from_be_bytes(header[..4].try_into().unwrap());
        let (size, header_size) = if short_size == 1 {
            let Some(bytes) = prefix.get(8..16) else {
                break;
            };
            (u64::from_be_bytes(bytes.try_into().unwrap()), 16)
        } else {
            (u64::from(short_size), 8)
        };
        let Some(next) = usize::try_from(size)
            .ok()
            .filter(|&size| size >= header_size)
            .and_then(|size| prefix.get(size..))
        else {
            break;
        };
        prefix = next;
    }
    true
}

fn looks_like_html(data: &[u8]) -> bool {
    let prefix = &data[..data.len().min(256)];
    let lower = String::from_utf8_lossy(prefix).to_ascii_lowercase();
    lower.contains("<!doctype html") || lower.contains("<html") || lower.contains("<head")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player_with_mailbox() -> (AudioPlayer, std::sync::mpsc::Receiver<BackendCommand>) {
        let mut player = AudioPlayer::new_without_audio();
        let (commands, receiver) = std::sync::mpsc::sync_channel(BACKEND_COMMAND_CAPACITY);
        player.backend = Some(BackendActor {
            commands,
            state: Arc::new(BackendState::new()),
            failures: Arc::default(),
            sample_tap: SampleTap::default(),
            thread: None,
        });
        (player, receiver)
    }

    #[test]
    fn retained_output_reactivates_for_new_tracks_and_retries_after_interruption() {
        let (mut player, commands) = player_with_mailbox();
        let available = Arc::new(AtomicBool::new(false));
        let session = available.clone();
        player.output_session = Some(Box::new(move |_| {
            anyhow::ensure!(session.load(Ordering::Acquire), "interrupted");
            Ok(())
        }));
        assert!(player.play_data(Arc::new(vec![1])).is_err());
        available.store(true, Ordering::Release);
        player.play_data(Arc::new(vec![2])).unwrap();
        assert!(
            commands
                .try_iter()
                .any(|c| matches!(c, BackendCommand::PlayData { .. }))
        );
        player.interrupt_output();
        available.store(false, Ordering::Release);
        assert!(player.resume_at(Duration::from_secs(4)).is_err());
        assert!(!player.has_audio());
        let (mut replacement, commands) = player_with_mailbox();
        player.backend = replacement.backend.take();
        player
            .restart_interrupted_input(Duration::from_secs(4))
            .unwrap();
        let BackendCommand::PlayData {
            playback_id,
            position,
            ..
        } = commands.try_recv().unwrap()
        else {
            panic!("Expected the interrupted track on the replacement output")
        };
        assert_eq!(playback_id, player.playback_id());
        assert_eq!(position, Duration::from_secs(4));
    }

    #[tokio::test]
    async fn stopping_playback_wakes_waiting_analysis() {
        let (mut player, _commands) = player_with_mailbox();
        let (events, _receiver) = mpsc::channel(1);
        player
            .play_url("http://127.0.0.1:1/stream", events, reqwest::Client::new())
            .unwrap();
        let wait = player.analysis_input().unwrap().wait();
        tokio::pin!(wait);
        assert!(futures::poll!(&mut wait).is_pending());

        player.stop();
        let std::task::Poll::Ready(result) = futures::poll!(&mut wait) else {
            panic!("analysis is still waiting after playback was cancelled");
        };
        assert_eq!(result.unwrap_err(), "Track changed");
    }

    #[test]
    fn deferred_output_session_is_idle_until_playback() {
        let activations = Arc::new(Mutex::new(Vec::new()));
        let recorded = activations.clone();
        let mut player = AudioPlayer::with_deferred_output(move |active| {
            super::super::lock_or_recover(&recorded).push(active);
            Err(anyhow!("Fixture declines audio activation"))
        });
        player.set_volume(0.35);
        player.stop();
        assert!(player.resume().is_err());
        assert!(!player.has_audio());
        assert_eq!(player.volume(), 0.35);
        assert!(super::super::lock_or_recover(&activations).is_empty());

        let error = player.play_data(Arc::new(vec![0; 4])).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Fixture declines audio activation")
        );
        assert_eq!(*super::super::lock_or_recover(&activations), [true]);
        assert!(!player.has_audio());
    }

    #[test]
    fn overlap_preparation_survives_file_resolution_but_explicit_stop_cancels_it() {
        let (mut player, commands) = player_with_mailbox();
        player.prepare_transition(Duration::from_secs(4));
        player.stop_for_preparation();
        player.play_data(Arc::new(vec![0; 4])).unwrap();
        assert_eq!(
            player
                .backend
                .as_ref()
                .unwrap()
                .state
                .transition_ms
                .load(Ordering::Acquire),
            4000
        );
        assert!(matches!(
            commands.try_recv(),
            Ok(BackendCommand::PlayData { .. })
        ));
        assert!(commands.try_recv().is_err());
        player.stop();
        assert_eq!(
            player
                .backend
                .as_ref()
                .unwrap()
                .state
                .transition_ms
                .load(Ordering::Acquire),
            0
        );
        assert!(matches!(commands.try_recv(), Ok(BackendCommand::Stop)));
    }

    #[test]
    fn buffered_drag_keeps_only_the_latest_target_and_stop_clears_it() {
        let (mut player, _commands) = player_with_mailbox();
        for position in 0..1_000 {
            player.seek(Duration::from_millis(position)).unwrap();
        }
        assert_eq!(
            super::super::lock_or_recover(&player.backend.as_ref().unwrap().state.pending_seek)
                .as_ref()
                .map(|request| (request.playback_id, request.position)),
            Some((player.playback_id(), Duration::from_millis(999)))
        );
        player.stop();
        assert!(
            super::super::lock_or_recover(&player.backend.as_ref().unwrap().state.pending_seek)
                .is_none()
        );
    }

    #[tokio::test]
    async fn mp4_with_trailing_movie_index_reaches_the_seekable_decoder() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let prefix = b"\0\0\0\x08ftyp\0\0\0\x0cmdatDATA";
        let index = b"\0\0\0\x08moov";
        assert!(!needs_indexed_mp4(
            Some("audio/mp4"),
            b"\0\0\0\x08ftyp\0\0\0\x08moov"
        ));
        assert!(needs_indexed_mp4(None, prefix));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/track", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket.read(&mut [0; 2048]).await.unwrap();
            socket.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: audio/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                prefix.len() + index.len()
            ).as_bytes()).await.unwrap();
            socket.write_all(prefix).await.unwrap();
            socket.write_all(index).await.unwrap();
        });
        let (mut player, commands) = player_with_mailbox();
        let (events, _) = mpsc::channel(8);
        player
            .play_url(&url, events, reqwest::Client::new())
            .unwrap();
        let id = player.playback_id();
        let file = tokio::task::spawn_blocking(move || {
            loop {
                match commands.recv_timeout(Duration::from_secs(3)).unwrap() {
                    BackendCommand::Stop => {}
                    BackendCommand::PlayData {
                        playback_id,
                        data: BufferedInput::File(file),
                        ..
                    } => {
                        assert_eq!(playback_id, id);
                        break file;
                    }
                    _ => panic!("MP4 with a trailing index must use a complete, seekable file"),
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(&file.path).unwrap(),
            [prefix.as_slice(), index].concat()
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn streaming_seeks_share_one_download_and_keep_only_the_latest_target() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/track", listener.local_addr().unwrap());
        let (started, ready) = tokio::sync::oneshot::channel();
        let (finish, release) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let length = socket.read(&mut request).await.unwrap();
            assert!(
                String::from_utf8_lossy(&request[..length])
                    .to_ascii_lowercase()
                    .contains("x-test-token: fixture")
            );
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: 8\r\nConnection: close\r\n\r\nRIFF").await.unwrap();
            started.send(()).unwrap();
            release.await.unwrap();
            socket.write_all(b"tail").await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(150), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (mut player, commands) = player_with_mailbox();
        let (events, _) = mpsc::channel(8);
        let mut headers = HeaderMap::new();
        headers.insert("x-test-token", "fixture".parse().unwrap());
        player
            .play_url_with_headers(
                &url,
                headers,
                None,
                events,
                reqwest::Client::builder().no_proxy().build().unwrap(),
            )
            .unwrap();
        ready.await.unwrap();
        let id = player.playback_id();
        let state = player.backend.as_ref().unwrap().state.clone();
        state.playback_id.store(id, Ordering::Release);
        state.mode.store(BACKEND_PLAYING, Ordering::Release);
        for seconds in [10, 20, 45] {
            player.seek(Duration::from_secs(seconds)).unwrap();
        }
        assert!(player.is_playing());
        assert_eq!(player.playback_id(), id);
        state.finished.store(true, Ordering::Release);
        assert!(!player.is_finished());
        assert!(
            super::super::lock_or_recover(&state.pending_seek)
                .as_ref()
                .unwrap()
                .input
                .is_none()
        );
        finish.send(()).unwrap();
        let file = tokio::time::timeout(
            Duration::from_secs(2),
            player.analysis_input().unwrap().wait(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(std::fs::read(&file.path).unwrap(), b"RIFFtail");
        assert_eq!(
            super::super::lock_or_recover(&state.pending_seek)
                .as_ref()
                .unwrap()
                .position,
            Duration::from_secs(45)
        );
        assert!(
            super::super::lock_or_recover(&state.pending_seek)
                .as_ref()
                .unwrap()
                .input
                .is_some()
        );
        player.pause();
        player.seek(Duration::from_secs(2)).unwrap();
        assert!(state.desired_paused.load(Ordering::Acquire));
        assert_eq!(player.playback_id(), id);
        server.await.unwrap();
        player.stop();
        drop(commands);
        drop(file);
    }

    #[tokio::test]
    async fn failed_stream_seek_reports_seek_failure_and_keeps_current_pipeline() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/track", listener.local_addr().unwrap());
        let (initial_sent, initial_ready) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nRIFF")
                .await
                .unwrap();
            initial_sent.send(()).unwrap();
            drop(socket);
            let (mut socket, _) = listener.accept().await.unwrap();
            socket.read(&mut request).await.unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let (mut player, _commands) = player_with_mailbox();
        let (events, mut failures) = mpsc::channel(8);
        player
            .play_url(
                &url,
                events,
                reqwest::Client::builder().no_proxy().build().unwrap(),
            )
            .unwrap();
        initial_ready.await.unwrap();
        let _file = tokio::time::timeout(
            Duration::from_secs(2),
            player.analysis_input().unwrap().wait(),
        )
        .await
        .unwrap()
        .unwrap();
        *super::super::lock_or_recover(&player.stream.as_ref().unwrap().seek) =
            RemoteSeek::Unprepared;

        let id = player.playback_id();
        player
            .backend
            .as_ref()
            .unwrap()
            .state
            .mode
            .store(BACKEND_PLAYING, Ordering::Release);
        player.seek(Duration::from_secs(30)).unwrap();
        assert!(
            matches!(tokio::time::timeout(Duration::from_secs(2), failures.recv()).await.unwrap(), Some(AudioEvent::SeekFailed {playback_id,..}) if playback_id == id)
        );
        assert_eq!(player.playback_id(), id);
        assert!(player.is_playing());
        assert!(
            super::super::lock_or_recover(&player.backend.as_ref().unwrap().state.pending_seek)
                .is_none()
        );
        assert!(matches!(
            *super::super::lock_or_recover(&player.stream.as_ref().unwrap().seek),
            RemoteSeek::Unprepared
        ));
        server.await.unwrap();
        player.stop();
    }

    #[tokio::test]
    async fn stopping_during_seek_download_cancels_without_starting_stale_audio() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/track", listener.local_addr().unwrap());
        let (started, downloading) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).await.unwrap();
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: 1024\r\nConnection: close\r\n\r\nRIFF").await.unwrap();
            started.send(()).unwrap();
            match tokio::time::timeout(Duration::from_secs(2), socket.read(&mut request))
                .await
                .unwrap()
            {
                Ok(0) => {}
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
                result => panic!("cancelled download must close its socket: {result:?}"),
            }
        });
        let (mut player, commands) = player_with_mailbox();
        let (events, mut failures) = mpsc::channel(8);
        player
            .play_url(
                &url,
                events,
                reqwest::Client::builder().no_proxy().build().unwrap(),
            )
            .unwrap();
        player.seek(Duration::from_secs(2)).unwrap();
        downloading.await.unwrap();
        player.stop();
        server.await.unwrap();
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok())
                .all(|command| !matches!(command, BackendCommand::PlayData { .. }))
        );
        assert!(failures.try_recv().is_err());
    }

    #[tokio::test]
    async fn stopping_during_seek_debounce_cancels_the_request() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/track", listener.local_addr().unwrap());
        let (mut player, _commands) = player_with_mailbox();
        let (events, _receiver) = mpsc::channel(8);
        player
            .play_url(&url, events, reqwest::Client::new())
            .unwrap();
        player.seek(Duration::from_secs(20)).unwrap();
        player.stop();
        assert!(player.stream.is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(250), listener.accept())
                .await
                .is_err()
        );
    }

    #[test]
    fn dropping_the_player_invalidates_download_and_backend_work() {
        let (mut player, commands) = player_with_mailbox();
        player.seek(Duration::from_secs(3)).unwrap();
        let generation = player.playback_generation.clone();
        let mut cancellation = player.cancellation_tx.subscribe();
        let state = player.backend.as_ref().unwrap().state.clone();
        let previous = player.playback_id();
        drop(player);
        assert!(generation.load(Ordering::Acquire) > previous);
        assert!(
            cancellation.has_changed().is_err(),
            "player drop closes its cancellation channel"
        );
        assert_eq!(
            *cancellation.borrow_and_update(),
            generation.load(Ordering::Acquire)
        );
        assert!(super::super::lock_or_recover(&state.pending_seek).is_none());
        assert!(state.shutdown.load(Ordering::Acquire));
        assert!(matches!(commands.try_recv(), Ok(BackendCommand::Stop)));
    }

    #[test]
    fn stop_invalidates_playback_generation() {
        let mut player = AudioPlayer::new_without_audio();
        let old_id = player.playback_id();
        player.stop();
        assert_ne!(old_id, player.playback_id());
    }

    #[test]
    fn redacts_plex_token_query_parameter() {
        let redacted = redact_url("https://example.test/audio?foo=1&X-Plex-Token=secret");
        assert!(redacted.contains("foo=1"));
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("X-Plex-Token"));
    }
}
