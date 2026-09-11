//! High-level audio player.
//!
//! Network fetching runs on Tokio tasks, decoder/device work lives on one
//! dedicated OS thread, and all completions carry a playback generation. The
//! UI thread only sends commands and reads atomics.

use super::rodio_backend::{
    BufferedInput, PipelineEvent, RodioBackend, SampleTap, StreamFailure, StreamingInput,
};
use super::traits::AudioBackend;
use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures::StreamExt;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

const MAX_AUDIO_BYTES: usize = 512 * 1024 * 1024;
const MAX_RETRIES: usize = 3;
const COMPRESSED_CHANNEL_CAPACITY: usize = 32;
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

enum BackendCommand {
    PlayData {
        playback_id: u64,
        data: BufferedInput,
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

struct BackendState {
    playback_id: AtomicU64,
    pending_seek: Mutex<Option<(u64, Duration)>>,
    mode: AtomicU8,
    position_ms: AtomicU64,
    finished: AtomicBool,
    volume_bits: AtomicU32,
    desired_paused: AtomicBool,
    shutdown: AtomicBool,
}

impl BackendState {
    fn new() -> Self {
        Self {
            playback_id: AtomicU64::new(0),
            pending_seek: Mutex::new(None),
            mode: AtomicU8::new(BACKEND_STOPPED),
            position_ms: AtomicU64::new(0),
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
                let mut active_playback_id = None;
                let mut applied_volume = backend.volume();
                loop {
                    if actor_state.shutdown.load(Ordering::Acquire) {
                        backend.stop();
                        break;
                    }
                    // Generation invalidation is the out-of-band high-priority
                    // stop path. It remains reliable even if the bounded
                    // command mailbox is temporarily full.
                    if active_playback_id.is_some_and(|id| generation.load(Ordering::Acquire) != id)
                    {
                        backend.stop();
                        active_playback_id = None;
                        stream_events = None;
                        actor_state.mode.store(BACKEND_STOPPED, Ordering::Release);
                        actor_state.position_ms.store(0, Ordering::Release);
                        actor_state.finished.store(false, Ordering::Release);
                    }

                    // Pause/resume is level-triggered rather than queued, so
                    // rapid input or a full playback-command mailbox cannot
                    // lose the user's final transport state.
                    if active_playback_id.is_some() {
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
                        Ok(BackendCommand::PlayData { playback_id, data }) => {
                            if generation.load(Ordering::Acquire) != playback_id {
                                continue;
                            }
                            actor_state.finished.store(false, Ordering::Release);
                            actor_state.position_ms.store(0, Ordering::Release);
                            actor_state
                                .playback_id
                                .store(playback_id, Ordering::Release);
                            stream_events = None;
                            match backend.start_buffered(data, Duration::ZERO) {
                                Ok(()) if generation.load(Ordering::Acquire) == playback_id => {
                                    actor_state.mode.store(BACKEND_PLAYING, Ordering::Release);
                                    active_playback_id = Some(playback_id);
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
                            if generation.load(Ordering::Acquire) != playback_id {
                                continue;
                            }
                            actor_state.finished.store(false, Ordering::Release);
                            actor_state.position_ms.store(0, Ordering::Release);
                            actor_state.mode.store(BACKEND_STOPPED, Ordering::Release);
                            actor_state
                                .playback_id
                                .store(playback_id, Ordering::Release);
                            match backend.start_stream(playback_id, input) {
                                Ok(()) => {
                                    active_playback_id = Some(playback_id);
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
                            active_playback_id = None;
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

                    // Coalesce buffered seeks too: a fast drag cannot fill the
                    // command queue or lose its final release position.
                    let seek = active_playback_id
                        .and_then(|_| super::lock_or_recover(&actor_state.pending_seek).take());
                    if let Some((playback_id, position)) = seek {
                        if generation.load(Ordering::Acquire) == playback_id
                            && active_playback_id == Some(playback_id)
                            && !backend.seek(position)
                        {
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
                                if let Some((event_id, events)) = &stream_events {
                                    if *event_id == playback_id {
                                        let _ = events
                                            .try_send(AudioEvent::BufferingReady { playback_id });
                                    }
                                }
                            }
                            PipelineEvent::Failed {
                                playback_id,
                                message,
                            } if generation.load(Ordering::Acquire) == playback_id => {
                                actor_state.mode.store(BACKEND_STOPPED, Ordering::Release);
                                active_playback_id = None;
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
struct StreamSession {
    url: String,
    headers: HeaderMap,
    fallback_url: Option<String>,
    events: mpsc::Sender<AudioEvent>,
    client: reqwest::Client,
}

pub struct AudioPlayer {
    backend: Option<BackendActor>,

    stream: Option<Arc<StreamSession>>,
    playback_generation: Arc<AtomicU64>,
    cancellation_tx: watch::Sender<u64>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let playback_generation = Arc::new(AtomicU64::new(0));
        let (cancellation_tx, _) = watch::channel(0);
        let backend = BackendActor::spawn(playback_generation.clone(), Duration::from_secs(5))?;
        Ok(Self {
            backend: Some(backend),
            stream: None,

            playback_generation,
            cancellation_tx,
        })
    }

    pub fn new_without_audio() -> Self {
        let (cancellation_tx, _) = watch::channel(0);
        Self {
            backend: None,
            stream: None,

            playback_generation: Arc::new(AtomicU64::new(0)),
            cancellation_tx,
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
        self.stop();
        self.stream = Some(Arc::new(StreamSession {
            url: url.to_string(),
            headers,
            fallback_url,
            events: event_tx,
            client: http_client,
        }));
        self.start_stream_at(Duration::ZERO, false)
    }

    fn start_stream_at(&self, position: Duration, debounce: bool) -> Result<()> {
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| anyhow!("No audio output device is available"))?;
        let backend_commands = backend.commands.clone();
        let session = self
            .stream
            .clone()
            .ok_or_else(|| anyhow!("No active stream"))?;
        let playback_id = self.playback_id();
        let generation = self.playback_generation.clone();
        let mut cancellation = self.cancellation_tx.subscribe();
        tokio::spawn(async move {
            // Mouse movement replaces, rather than queues, stream restarts.
            if debounce {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(150)) => {}
                    _ = wait_for_cancellation(&mut cancellation, playback_id) => return,
                }
            }
            stream_audio(
                StreamRequest {
                    primary_url: &session.url,
                    fallback_url: session.fallback_url.as_deref(),
                    headers: &session.headers,
                    client: &session.client,
                    start_position: position,
                },
                playback_id,
                &generation,
                &mut cancellation,
                &backend_commands,
                &session.events,
            )
            .await;
        });
        Ok(())
    }

    pub fn has_audio(&self) -> bool {
        self.backend.is_some()
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
        self.backend = Some(BackendActor::spawn(
            self.playback_generation.clone(),
            Duration::from_secs(5),
        )?);
        Ok(true)
    }

    pub fn play_data(&mut self, data: Arc<Vec<u8>>) -> Result<()> {
        self.play_input(data.into())
    }

    pub fn play_file(&mut self, file: crate::library::MediaFile) -> Result<()> {
        self.play_input(BufferedInput::File(file))
    }

    fn play_input(&mut self, data: BufferedInput) -> Result<()> {
        self.stop();
        let playback_id = self.playback_id();
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| anyhow!("No audio output device is available"))?;
        backend.send(BackendCommand::PlayData { playback_id, data })
    }

    pub fn pause(&mut self) {
        if let Some(backend) = &self.backend {
            backend.state.desired_paused.store(true, Ordering::Release);
        }
    }

    pub fn resume(&mut self) {
        if let Some(backend) = &self.backend {
            backend.state.desired_paused.store(false, Ordering::Release);
        }
    }

    pub fn stop(&mut self) {
        self.stream = None;
        self.invalidate_playback();
        if let Some(backend) = &self.backend {
            backend.state.desired_paused.store(false, Ordering::Release);
            *super::lock_or_recover(&backend.state.pending_seek) = None;
            let _ = backend.send(BackendCommand::Stop);
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        if let Some(backend) = &self.backend {
            backend
                .state
                .volume_bits
                .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Release);
        }
    }

    pub fn volume(&self) -> f32 {
        self.backend.as_ref().map_or(0.8, |backend| {
            f32::from_bits(backend.state.volume_bits.load(Ordering::Acquire))
        })
    }

    pub fn is_finished(&self) -> bool {
        self.backend.as_ref().is_some_and(|backend| {
            backend.state.playback_id.load(Ordering::Acquire) == self.playback_id()
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
        if self.stream.is_some() {
            // New generation rejects old decoder/network completions. Do not
            // use stop(): seeking must retain the stream request and pause state.
            self.invalidate_playback();
            return self.start_stream_at(position, true);
        }
        *super::lock_or_recover(&backend.state.pending_seek) = Some((self.playback_id(), position));
        Ok(())
    }

    pub fn position(&self) -> Option<Duration> {
        self.backend
            .as_ref()
            .filter(|backend| {
                backend.state.playback_id.load(Ordering::Acquire) == self.playback_id()
                    && backend.state.mode.load(Ordering::Acquire) != BACKEND_STOPPED
            })
            .map(|backend| Duration::from_millis(backend.state.position_ms.load(Ordering::Acquire)))
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
}

async fn stream_audio(
    request: StreamRequest<'_>,
    playback_id: u64,
    generation: &AtomicU64,
    cancellation: &mut watch::Receiver<u64>,
    backend_commands: &std::sync::mpsc::SyncSender<BackendCommand>,
    events: &mpsc::Sender<AudioEvent>,
) {
    let StreamRequest {
        primary_url,
        fallback_url,
        headers,
        client,
        start_position,
    } = request;
    let mut last_error = "request cancelled".to_string();

    for url in [Some(primary_url), fallback_url].into_iter().flatten() {
        tracing::debug!("Opening audio stream: {}", redact_url(url));
        for attempt in 0..MAX_RETRIES {
            if !is_current(generation, playback_id) {
                return;
            }

            let response = tokio::select! {
                response = client.get(url).headers(headers.clone()).send() => response,
                _ = wait_for_cancellation(cancellation, playback_id) => return,
            };

            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    last_error = if error.is_timeout() {
                        "audio server timed out".to_string()
                    } else if error.is_connect() {
                        "could not connect to the audio server".to_string()
                    } else {
                        "audio request failed".to_string()
                    };
                    if attempt + 1 == MAX_RETRIES {
                        break;
                    }
                    let delay = Duration::from_secs(1_u64 << attempt);
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = wait_for_cancellation(cancellation, playback_id) => return,
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
                    _ = wait_for_cancellation(cancellation, playback_id) => return,
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
                chunk = body.next() => chunk,
                _ = wait_for_cancellation(cancellation, playback_id) => return,
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

            if looks_like_html(&first) {
                last_error = "server returned HTML instead of audio".to_string();
                break;
            }

            let (compressed_tx, compressed_rx) =
                mpsc::channel::<Bytes>(COMPRESSED_CHANNEL_CAPACITY);
            let failure = StreamFailure::default();
            let input =
                StreamingInput::new(compressed_rx, failure.clone(), mime_type, start_position);
            if backend_commands
                .try_send(BackendCommand::PlayStream {
                    playback_id,
                    input,
                    events: events.clone(),
                })
                .is_err()
            {
                last_error = "audio backend is unavailable".to_string();
                break;
            }

            let mut transferred = first.len();
            let first_send = tokio::select! {
                result = compressed_tx.send(first) => result,
                _ = wait_for_cancellation(cancellation, playback_id) => return,
            };
            if first_send.is_err() {
                return;
            }

            loop {
                let next = tokio::select! {
                    chunk = body.next() => chunk,
                    _ = wait_for_cancellation(cancellation, playback_id) => return,
                };
                let Some(chunk) = next else {
                    // Dropping the sender communicates clean EOF.
                    return;
                };
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(_) => {
                        failure.set("audio transfer was interrupted".to_string());
                        return;
                    }
                };
                transferred = transferred.saturating_add(chunk.len());
                if transferred > MAX_AUDIO_BYTES {
                    failure.set(format!(
                        "audio response exceeded {} MiB limit",
                        MAX_AUDIO_BYTES / (1024 * 1024)
                    ));
                    return;
                }
                let send = tokio::select! {
                    result = compressed_tx.send(chunk) => result,
                    _ = wait_for_cancellation(cancellation, playback_id) => return,
                };
                if send.is_err() {
                    return;
                }
            }
        }
        tracing::warn!(
            "Audio endpoint {} failed before streaming; trying fallback if available",
            redact_url(url)
        );
    }

    if is_current(generation, playback_id) {
        let _ = events
            .send(AudioEvent::Error {
                playback_id,
                message: format!("Playback failed: {last_error}"),
            })
            .await;
    }
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
    fn buffered_drag_keeps_only_the_latest_target_and_stop_clears_it() {
        let (mut player, _commands) = player_with_mailbox();
        for position in 0..1_000 {
            player.seek(Duration::from_millis(position)).unwrap();
        }
        assert_eq!(
            *super::super::lock_or_recover(&player.backend.as_ref().unwrap().state.pending_seek),
            Some((player.playback_id(), Duration::from_millis(999)))
        );
        player.stop();
        assert!(super::super::lock_or_recover(
            &player.backend.as_ref().unwrap().state.pending_seek
        )
        .is_none());
    }

    #[tokio::test]
    async fn streamed_drag_reopens_once_at_latest_target_and_preserves_pause() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/track", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let len = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..len]).to_ascii_lowercase();
            assert!(request.contains("x-test-token: fixture"));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: 4\r\nConnection: close\r\n\r\nRIFF").await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(250), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (mut player, commands) = player_with_mailbox();
        let (events, _receiver) = mpsc::channel(8);
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
        player.pause();
        let initial_id = player.playback_id();
        for millis in [10_000, 20_000, 30_000] {
            player.seek(Duration::from_millis(millis)).unwrap();
        }
        assert_ne!(initial_id, player.playback_id());
        assert!(player
            .backend
            .as_ref()
            .unwrap()
            .state
            .desired_paused
            .load(Ordering::Acquire));
        assert!(player.position().is_none()); // Old actor position must not overwrite the seek.
        assert!(!player.is_finished());
        server.await.unwrap();
        let mut streams = 0;
        while let Ok(command) = commands.try_recv() {
            if let BackendCommand::PlayStream {
                playback_id, input, ..
            } = command
            {
                streams += 1;
                assert_eq!(playback_id, player.playback_id());
                assert_eq!(input.start_position, Duration::from_secs(30));
            }
        }
        assert_eq!(streams, 1);
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
