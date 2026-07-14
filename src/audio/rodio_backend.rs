//! Rodio audio backend with an isolated decoder worker.
//!
//! Compressed bytes are decoded ahead of the device callback into a bounded
//! lock-free PCM ring. The real-time callback never waits for network, disk,
//! decoder, or visualization locks; it consumes PCM or emits silence during a
//! genuine underrun.

use super::traits::{AudioBackend, AudioError};
use bytes::Bytes;
use ringbuf::{traits::*, HeapCons, HeapProd, HeapRb};
use rodio::source::Source;
use rodio::{
    cpal::traits::{DeviceTrait, HostTrait}, ChannelCount, Decoder, OutputStream,
    OutputStreamBuilder, Sample, SampleRate, Sink,
};
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::mpsc as tokio_mpsc;

const SAMPLE_TAP_CAP: usize = 4_096;
const PCM_BUFFER_SECONDS: usize = 8;
const PCM_PREBUFFER_MS: usize = 750;

/// Error slot shared by the asynchronous HTTP producer and blocking decoder.
/// The decoder sees channel closure as EOF; this separate slot distinguishes a
/// clean end-of-stream from a failed transfer without placing HTTP work on the
/// audio thread.
#[derive(Clone, Default)]
pub(crate) struct StreamFailure(Arc<Mutex<Option<String>>>);

impl StreamFailure {
    pub(crate) fn set(&self, message: String) {
        *super::lock_or_recover(&self.0) = Some(message);
    }

    fn message(&self) -> Option<String> {
        super::lock_or_recover(&self.0).clone()
    }
}

/// Bounded compressed-byte stream consumed by the decoder worker.
pub(crate) struct StreamingInput {
    receiver: tokio_mpsc::Receiver<Bytes>,
    failure: StreamFailure,
    mime_type: Option<String>,
}

impl StreamingInput {
    pub(crate) fn new(
        receiver: tokio_mpsc::Receiver<Bytes>,
        failure: StreamFailure,
        mime_type: Option<String>,
    ) -> Self {
        Self {
            receiver,
            failure,
            mime_type,
        }
    }
}

/// `rodio::Decoder` requires `Read + Seek + Send + Sync` even for a source
/// advertised as non-seekable. Reads wait on the bounded Tokio channel from a
/// dedicated decoder thread. The short polling interval makes cancellation
/// deterministic, so stopping playback cannot hang while joining this worker.
struct StreamingReader {
    receiver: Mutex<tokio_mpsc::Receiver<Bytes>>,
    current: Cursor<Bytes>,
    position: u64,
    failure: StreamFailure,
    cancelled: Arc<AtomicBool>,
}

impl StreamingReader {
    fn new(input: StreamingInput, cancelled: Arc<AtomicBool>) -> (Self, Option<String>) {
        let mime_type = input.mime_type;
        (
            Self {
                receiver: Mutex::new(input.receiver),
                current: Cursor::new(Bytes::new()),
                position: 0,
                failure: input.failure,
                cancelled,
            },
            mime_type,
        )
    }
}

impl Read for StreamingReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }

        loop {
            let read = self.current.read(output)?;
            if read > 0 {
                self.position = self.position.saturating_add(read as u64);
                return Ok(read);
            }
            if self.cancelled.load(Ordering::Acquire) {
                return Ok(0);
            }

            let next = super::lock_or_recover(&self.receiver).try_recv();
            match next {
                Ok(chunk) if !chunk.is_empty() => {
                    self.current = Cursor::new(chunk);
                }
                Ok(_) | Err(tokio_mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(tokio_mpsc::error::TryRecvError::Disconnected) => {
                    if let Some(message) = self.failure.message() {
                        return Err(io::Error::other(message));
                    }
                    return Ok(0);
                }
            }
        }
    }
}

impl Seek for StreamingReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        match position {
            SeekFrom::Current(0) => Ok(self.position),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "live audio stream is not seekable",
            )),
        }
    }
}

enum PipelineMessage {
    Ready { playback_id: u64, sink: Sink },
    Failed { playback_id: u64, message: String },
}

pub(crate) enum PipelineEvent {
    Ready { playback_id: u64 },
    Failed { playback_id: u64, message: String },
}

/// UI-facing consumer for recent stereo sample pairs.
///
/// The audio callback owns the corresponding lock-free producer directly.
/// The mutex is touched only by the UI while swapping/draining consumers, so
/// UI rendering cannot block the device callback.
#[derive(Clone, Default)]
pub struct SampleTap {
    consumer: Arc<Mutex<Option<HeapCons<(f32, f32)>>>>,
}

impl std::fmt::Debug for SampleTap {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SampleTap(..)")
    }
}

impl SampleTap {
    fn install(&self, consumer: HeapCons<(f32, f32)>) {
        *super::lock_or_recover(&self.consumer) = Some(consumer);
    }

    pub fn clear(&self) {
        if let Some(consumer) = super::lock_or_recover(&self.consumer).as_mut() {
            while consumer.try_pop().is_some() {}
        }
    }

    /// Drain at most `limit` pairs into `output`.
    pub fn drain_into(&self, output: &mut Vec<(f32, f32)>, limit: usize) {
        let mut guard = super::lock_or_recover(&self.consumer);
        let Some(consumer) = guard.as_mut() else {
            return;
        };
        output.reserve(limit.min(consumer.occupied_len()));
        for _ in 0..limit {
            let Some(pair) = consumer.try_pop() else {
                break;
            };
            output.push(pair);
        }
    }
}

struct TapSource<S> {
    inner: S,
    producer: HeapProd<(f32, f32)>,
    pending_left: Option<f32>,
    is_stereo: bool,
}

impl<S> TapSource<S>
where
    S: Source<Item = Sample>,
{
    fn new(inner: S, tap: &SampleTap) -> Self {
        let ring = HeapRb::new(SAMPLE_TAP_CAP);
        let (producer, consumer) = ring.split();
        tap.install(consumer);
        let is_stereo = inner.channels() >= 2;
        Self {
            inner,
            producer,
            pending_left: None,
            is_stereo,
        }
    }

    #[inline]
    fn push_pair(&mut self, pair: (f32, f32)) {
        // A full visualization ring is intentionally lossy. Audio must never
        // wait for the UI to drain it.
        let _ = self.producer.try_push(pair);
    }
}

impl<S> Iterator for TapSource<S>
where
    S: Source<Item = Sample>,
{
    type Item = Sample;

    #[inline]
    fn next(&mut self) -> Option<Sample> {
        let sample = self.inner.next()?;
        if self.is_stereo {
            match self.pending_left.take() {
                Some(left) => self.push_pair((left, sample)),
                None => self.pending_left = Some(sample),
            }
        } else {
            self.push_pair((sample, sample));
        }
        Some(sample)
    }
}

impl<S> Source for TapSource<S>
where
    S: Source<Item = Sample>,
{
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

/// Nonblocking source read by rodio's mixer callback.
struct PcmSource {
    consumer: HeapCons<Sample>,
    finished: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    underruns: Arc<AtomicU64>,
    channels: ChannelCount,
    sample_rate: SampleRate,
    duration: Option<Duration>,
}

impl Iterator for PcmSource {
    type Item = Sample;

    #[inline]
    fn next(&mut self) -> Option<Sample> {
        if let Some(sample) = self.consumer.try_pop() {
            return Some(sample);
        }
        if self.cancelled.load(Ordering::Acquire)
            || self.finished.load(Ordering::Acquire)
        {
            return None;
        }

        self.underruns.fetch_add(1, Ordering::Relaxed);
        Some(0.0)
    }
}

impl Source for PcmSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.duration
    }
}

#[derive(Clone)]
struct SharedBytes(Arc<Vec<u8>>);

impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

struct DecoderFinished(Arc<AtomicBool>);

impl Drop for DecoderFinished {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

pub struct RodioBackend {
    stream: OutputStream,
    sink: Option<Sink>,
    volume: f32,
    tap: SampleTap,
    decoder_cancelled: Option<Arc<AtomicBool>>,
    decoder_thread: Option<JoinHandle<()>>,
    current_data: Option<Arc<Vec<u8>>>,
    base_position: Duration,
    underruns: Arc<AtomicU64>,
    pipeline_tx: std_mpsc::SyncSender<PipelineMessage>,
    pipeline_rx: std_mpsc::Receiver<PipelineMessage>,
    active_playback_id: Option<u64>,
}

impl RodioBackend {
    pub fn new() -> Result<Self, AudioError> {
        let mut stream = match OutputStreamBuilder::open_default_stream() {
            Ok(stream) => stream,
            Err(default_error) => {
                let host = rodio::cpal::default_host();
                tracing::error!("audio: default stream failed: {}", default_error);
                let mut last_error = default_error.to_string();
                let mut opened = None;
                let devices = host.output_devices().map_err(|error| {
                    AudioError::NoDevice(format!("host.output_devices() failed: {error}"))
                })?;

                for device in devices {
                    let name = device
                        .name()
                        .unwrap_or_else(|_| "<unnamed>".to_string());
                    match OutputStreamBuilder::from_device(device)
                        .and_then(|builder| builder.open_stream_or_fallback())
                    {
                        Ok(stream) => {
                            tracing::info!("audio: opened fallback output device '{}'", name);
                            opened = Some(stream);
                            break;
                        }
                        Err(error) => {
                            tracing::warn!("audio: fallback device '{}' failed: {}", name, error);
                            last_error = format!("{name}: {error}");
                        }
                    }
                }

                opened.ok_or(AudioError::NoDevice(last_error))?
            }
        };
        stream.log_on_drop(false);

        // A decoder emits at most Ready plus Failed. Keeping this bounded
        // prevents a detached or buggy worker from growing actor memory.
        let (pipeline_tx, pipeline_rx) = std_mpsc::sync_channel(2);
        Ok(Self {
            stream,
            sink: None,
            volume: 0.8,
            tap: SampleTap::default(),
            decoder_cancelled: None,
            decoder_thread: None,
            current_data: None,
            base_position: Duration::ZERO,
            underruns: Arc::new(AtomicU64::new(0)),
            pipeline_tx,
            pipeline_rx,
            active_playback_id: None,
        })
    }

    pub fn sample_tap(&self) -> SampleTap {
        self.tap.clone()
    }

    fn try_decode(
        data: Arc<Vec<u8>>,
    ) -> Result<Decoder<Cursor<SharedBytes>>, AudioError> {
        let byte_len = data.len() as u64;
        match crate::util::catch_expected_panic(move || {
            Decoder::builder()
                .with_data(Cursor::new(SharedBytes(data)))
                .with_byte_len(byte_len)
                .build()
        }) {
            Ok(Ok(source)) => Ok(source),
            Ok(Err(error)) => Err(AudioError::DecodeError(error.to_string())),
            Err(_) => Err(AudioError::DecodeError(
                "decoder crashed on unsupported or corrupt media".to_string(),
            )),
        }
    }

    fn try_decode_stream(
        input: StreamingInput,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Decoder<StreamingReader>, AudioError> {
        let (reader, mime_type) = StreamingReader::new(input, cancelled);
        match crate::util::catch_expected_panic(move || {
            let builder = Decoder::builder()
                .with_data(reader)
                .with_seekable(false);
            match mime_type {
                Some(mime_type) => builder.with_mime_type(&mime_type).build(),
                None => builder.build(),
            }
        }) {
            Ok(Ok(source)) => Ok(source),
            Ok(Err(error)) => Err(AudioError::DecodeError(error.to_string())),
            Err(_) => Err(AudioError::DecodeError(
                "decoder crashed on unsupported or corrupt media".to_string(),
            )),
        }
    }

    fn stop_pipeline(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        if let Some(cancelled) = self.decoder_cancelled.take() {
            cancelled.store(true, Ordering::Release);
        }
        if let Some(handle) = self.decoder_thread.take() {
            let _ = handle.join();
        }
        self.tap.clear();
        self.active_playback_id = None;
        while self.pipeline_rx.try_recv().is_ok() {}
    }

    fn start_buffered(
        &mut self,
        data: Arc<Vec<u8>>,
        start_position: Duration,
    ) -> Result<(), AudioError> {
        self.stop_pipeline();

        let mut decoder = Self::try_decode(data.clone())?;
        if !start_position.is_zero() {
            decoder
                .try_seek(start_position)
                .map_err(|error| AudioError::SeekError(error.to_string()))?;
        }

        let channels = decoder.channels();
        let sample_rate = decoder.sample_rate();
        let duration = decoder
            .total_duration()
            .map(|total| total.saturating_sub(start_position));
        let samples_per_second = sample_rate as usize * channels as usize;
        let capacity = samples_per_second
            .saturating_mul(PCM_BUFFER_SECONDS)
            .max(8_192);
        let prebuffer_samples = samples_per_second
            .saturating_mul(PCM_PREBUFFER_MS)
            / 1_000;

        let ring = HeapRb::new(capacity);
        let (mut producer, consumer) = ring.split();

        // Build a short PCM runway before connecting the source to the device.
        // This work happens on the audio actor, never the UI thread.
        let prebuffer_result = crate::util::catch_expected_panic(|| {
            for _ in 0..prebuffer_samples {
                let Some(sample) = decoder.next() else {
                    break;
                };
                if producer.try_push(sample).is_err() {
                    break;
                }
            }
        });
        if prebuffer_result.is_err() {
            return Err(AudioError::DecodeError(
                "decoder crashed while prebuffering media".to_string(),
            ));
        }

        let finished = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_finished = finished.clone();
        let worker_cancelled = cancelled.clone();
        let decoder_thread = std::thread::Builder::new()
            .name("textamp-audio-decoder".to_string())
            .spawn(move || {
                let _finished_guard = DecoderFinished(worker_finished);
                let _ = crate::util::catch_expected_panic(move || loop {
                    if worker_cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    match decoder.next() {
                        Some(sample) => {
                            let mut sample = sample;
                            loop {
                                match producer.try_push(sample) {
                                    Ok(()) => break,
                                    Err(returned) => {
                                        if worker_cancelled.load(Ordering::Acquire) {
                                            return;
                                        }
                                        sample = returned;
                                        std::thread::sleep(Duration::from_millis(1));
                                    }
                                }
                            }
                        }
                        None => break,
                    }
                });
            })
            .map_err(|error| AudioError::PlaybackError(error.to_string()))?;

        let source = PcmSource {
            consumer,
            finished,
            cancelled: cancelled.clone(),
            underruns: self.underruns.clone(),
            channels,
            sample_rate,
            duration,
        };
        let tapped = TapSource::new(source, &self.tap);
        let sink = Sink::connect_new(self.stream.mixer());
        sink.set_volume(self.volume);
        sink.append(tapped);

        self.sink = Some(sink);
        self.decoder_cancelled = Some(cancelled);
        self.decoder_thread = Some(decoder_thread);
        self.current_data = Some(data);
        self.base_position = start_position;
        Ok(())
    }

    /// Start an incremental compressed stream. Decoder construction and PCM
    /// prebuffering happen on the decoder worker, so the actor remains able to
    /// process Stop/Pause commands even when the network stalls.
    pub(crate) fn start_stream(
        &mut self,
        playback_id: u64,
        input: StreamingInput,
    ) -> Result<(), AudioError> {
        self.stop_pipeline();

        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let pipeline_tx = self.pipeline_tx.clone();
        let mixer = self.stream.mixer().clone();
        let tap = self.tap.clone();
        let volume = self.volume;
        let underruns = self.underruns.clone();
        let failure = input.failure.clone();

        let decoder_thread = std::thread::Builder::new()
            .name("textamp-stream-decoder".to_string())
            .spawn(move || {
                let run_cancelled = worker_cancelled.clone();
                let run_failure = failure.clone();
                let ready_tx = pipeline_tx.clone();
                let result = crate::util::catch_expected_panic(move || -> Result<(), AudioError> {
                    let mut decoder = Self::try_decode_stream(input, run_cancelled.clone())?;
                    let channels = decoder.channels();
                    let sample_rate = decoder.sample_rate();
                    let samples_per_second = sample_rate as usize * channels as usize;
                    let capacity = samples_per_second
                        .saturating_mul(PCM_BUFFER_SECONDS)
                        .max(8_192);
                    let prebuffer_samples = samples_per_second
                        .saturating_mul(PCM_PREBUFFER_MS)
                        / 1_000;
                    let ring = HeapRb::new(capacity);
                    let (mut producer, consumer) = ring.split();

                    for _ in 0..prebuffer_samples {
                        if run_cancelled.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        let Some(sample) = decoder.next() else {
                            if let Some(message) = run_failure.message() {
                                return Err(AudioError::PlaybackError(message));
                            }
                            break;
                        };
                        if producer.try_push(sample).is_err() {
                            break;
                        }
                    }

                    if run_cancelled.load(Ordering::Acquire) {
                        return Ok(());
                    }

                    let finished = Arc::new(AtomicBool::new(false));
                    let _finished_guard = DecoderFinished(finished.clone());
                    let source = PcmSource {
                        consumer,
                        finished,
                        cancelled: run_cancelled.clone(),
                        underruns,
                        channels,
                        sample_rate,
                        duration: None,
                    };
                    let tapped = TapSource::new(source, &tap);
                    let sink = Sink::connect_new(&mixer);
                    sink.set_volume(volume);
                    sink.append(tapped);
                    if ready_tx
                        .send(PipelineMessage::Ready { playback_id, sink })
                        .is_err()
                    {
                        return Ok(());
                    }

                    loop {
                        if run_cancelled.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        match decoder.next() {
                            Some(sample) => {
                                let mut sample = sample;
                                loop {
                                    match producer.try_push(sample) {
                                        Ok(()) => break,
                                        Err(returned) => {
                                            if run_cancelled.load(Ordering::Acquire) {
                                                return Ok(());
                                            }
                                            sample = returned;
                                            std::thread::sleep(Duration::from_millis(1));
                                        }
                                    }
                                }
                            }
                            None => {
                                if let Some(message) = run_failure.message() {
                                    return Err(AudioError::PlaybackError(message));
                                }
                                return Ok(());
                            }
                        }
                    }
                });

                let message = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error.to_string()),
                    Err(_) => Some("decoder crashed while streaming media".to_string()),
                };
                if !worker_cancelled.load(Ordering::Acquire) {
                    if let Some(message) = message {
                        let _ = pipeline_tx.send(PipelineMessage::Failed {
                            playback_id,
                            message,
                        });
                    }
                }
            })
            .map_err(|error| AudioError::PlaybackError(error.to_string()))?;

        self.decoder_cancelled = Some(cancelled);
        self.decoder_thread = Some(decoder_thread);
        self.current_data = None;
        self.base_position = Duration::ZERO;
        self.active_playback_id = Some(playback_id);
        Ok(())
    }

    pub(crate) fn poll_pipeline(&mut self) -> Vec<PipelineEvent> {
        let mut events = Vec::new();
        while let Ok(message) = self.pipeline_rx.try_recv() {
            match message {
                PipelineMessage::Ready { playback_id, sink }
                    if self.active_playback_id == Some(playback_id) =>
                {
                    self.sink = Some(sink);
                    events.push(PipelineEvent::Ready { playback_id });
                }
                PipelineMessage::Ready { sink, .. } => sink.stop(),
                PipelineMessage::Failed {
                    playback_id,
                    message,
                } if self.active_playback_id == Some(playback_id) => {
                    if let Some(sink) = self.sink.take() {
                        sink.stop();
                    }
                    events.push(PipelineEvent::Failed {
                        playback_id,
                        message,
                    });
                }
                PipelineMessage::Failed { .. } => {}
            }
        }
        events
    }

    pub fn underrun_count(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }
}

impl Drop for RodioBackend {
    fn drop(&mut self) {
        self.stop_pipeline();
    }
}

impl AudioBackend for RodioBackend {
    fn play_data(&mut self, data: Arc<Vec<u8>>) -> Result<(), AudioError> {
        self.start_buffered(data, Duration::ZERO)
    }

    fn pause(&mut self) {
        if let Some(sink) = &self.sink {
            sink.pause();
        }
    }

    fn resume(&mut self) {
        if let Some(sink) = &self.sink {
            sink.play();
        }
    }

    fn stop(&mut self) {
        self.stop_pipeline();
        self.current_data = None;
        self.base_position = Duration::ZERO;
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(sink) = &self.sink {
            sink.set_volume(self.volume);
        }
    }

    fn volume(&self) -> f32 {
        self.volume
    }

    fn is_finished(&self) -> bool {
        self.sink.as_ref().is_some_and(Sink::empty)
    }

    fn is_playing(&self) -> bool {
        self.sink
            .as_ref()
            .is_some_and(|sink| !sink.is_paused() && !sink.empty())
    }

    fn is_paused(&self) -> bool {
        self.sink.as_ref().is_some_and(Sink::is_paused)
    }

    fn seek(&mut self, position: Duration) -> bool {
        let Some(data) = self.current_data.clone() else {
            return false;
        };
        match self.start_buffered(data, position) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!("Seek failed: {}", error);
                false
            }
        }
    }

    fn position(&self) -> Option<Duration> {
        self.sink
            .as_ref()
            .map(|sink| self.base_position.saturating_add(sink.get_pos()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_reader_concatenates_bounded_chunks() {
        let (sender, receiver) = tokio_mpsc::channel(2);
        sender.try_send(Bytes::from_static(b"abc")).unwrap();
        sender.try_send(Bytes::from_static(b"def")).unwrap();
        drop(sender);

        let input = StreamingInput::new(receiver, StreamFailure::default(), None);
        let (mut reader, _) = StreamingReader::new(input, Arc::new(AtomicBool::new(false)));
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();

        assert_eq!(output, b"abcdef");
        assert_eq!(reader.seek(SeekFrom::Current(0)).unwrap(), 6);
        assert_eq!(
            reader.seek(SeekFrom::Start(0)).unwrap_err().kind(),
            io::ErrorKind::Unsupported,
        );
    }

    #[test]
    fn streaming_reader_cancellation_does_not_wait_for_network() {
        let (_sender, receiver) = tokio_mpsc::channel(1);
        let cancelled = Arc::new(AtomicBool::new(true));
        let input = StreamingInput::new(receiver, StreamFailure::default(), None);
        let (mut reader, _) = StreamingReader::new(input, cancelled);
        let mut output = [0; 8];

        assert_eq!(reader.read(&mut output).unwrap(), 0);
    }

    #[test]
    fn streaming_reader_propagates_transfer_failure() {
        let (sender, receiver) = tokio_mpsc::channel(1);
        let failure = StreamFailure::default();
        failure.set("network interrupted".to_string());
        drop(sender);
        let input = StreamingInput::new(receiver, failure, None);
        let (mut reader, _) = StreamingReader::new(input, Arc::new(AtomicBool::new(false)));
        let mut output = [0; 8];

        let error = reader.read(&mut output).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(error.to_string(), "network interrupted");
    }
}
