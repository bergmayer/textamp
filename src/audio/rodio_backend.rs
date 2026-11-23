//! Rodio audio backend with an isolated decoder worker.
//!
//! Compressed bytes are decoded ahead of the device callback into a bounded
//! lock-free PCM ring. The real-time callback never waits for network, disk,
//! decoder, or visualization locks; it consumes PCM or emits silence during a
//! genuine underrun.

use super::traits::{AudioBackend, AudioError};
use ringbuf::{HeapCons, HeapProd, HeapRb, traits::*};
use rodio::source::Source;
use rodio::{ChannelCount, Decoder, OutputStream, OutputStreamBuilder, Sample, SampleRate, Sink};
use std::io::{Cursor, Read, Seek};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Reopenable compressed input. File-backed playback uses the same decoder,
/// bounded PCM pipeline, and seek path as cached bytes, without loading the
/// whole file into RAM. Temporary remote files live as long as this input.
#[derive(Clone, Debug)]
pub(crate) enum BufferedInput {
    Bytes(Arc<Vec<u8>>),
    File(crate::library::MediaFile),
}
impl From<Arc<Vec<u8>>> for BufferedInput {
    fn from(bytes: Arc<Vec<u8>>) -> Self {
        Self::Bytes(bytes)
    }
}
pub(crate) trait AudioReader: Read + Seek + Send + Sync {}
impl<T: Read + Seek + Send + Sync> AudioReader for T {}

const SAMPLE_TAP_CAP: usize = 4_096;
const PCM_BUFFER_SECONDS: usize = 8;
const PCM_PREBUFFER_MS: usize = 750;

/// Error slot shared by the asynchronous HTTP producer and blocking decoder.
/// This separate slot distinguishes a
/// clean end-of-stream from a failed transfer without placing HTTP work on the
/// audio thread.
#[derive(Clone, Default)]
pub(crate) struct StreamFailure(Arc<Mutex<Option<String>>>);

impl StreamFailure {
    pub(crate) fn set(&self, message: String) {
        *super::lock_or_recover(&self.0) = Some(message);
    }

    pub(crate) fn message(&self) -> Option<String> {
        super::lock_or_recover(&self.0).clone()
    }
}

/// Progressive disk-backed compressed input, retaining one download for playback and seeks.
pub(crate) struct StreamingInput {
    spool: Arc<super::spool::Spool>,
    pub(crate) failure: StreamFailure,
    mime_type: Option<String>,
    pub(crate) start_position: Duration,
}
impl StreamingInput {
    pub(crate) fn from_spool(
        spool: Arc<super::spool::Spool>,
        failure: StreamFailure,
        mime_type: Option<String>,
        start_position: Duration,
    ) -> Self {
        Self {
            spool,
            failure,
            mime_type,
            start_position,
        }
    }
}
/// Decode bounded blocks on the worker and publish each block in one ring update.
fn fill_pcm(
    decoder: &mut impl Iterator<Item = Sample>,
    producer: &mut HeapProd<Sample>,
    cancelled: &AtomicBool,
) {
    let mut block = [0.; 1024];
    loop {
        if cancelled.load(Ordering::Acquire) {
            return;
        }
        let count = producer.vacant_len().min(block.len());
        if count == 0 {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        let mut decoded = 0;
        for slot in &mut block[..count] {
            let Some(sample) = decoder.next() else {
                break;
            };
            *slot = sample;
            decoded += 1;
        }
        producer.push_slice(&block[..decoded]);
        if decoded < count {
            return;
        }
    }
}

/// Non-seekable HTTP bodies must be decoded up to the requested time. This
/// runs on the cancellable decoder worker with bounded buffers, never the UI
/// or audio callback. Cached tracks still use the decoder's indexed seek.
pub(crate) fn skip_stream_to(
    decoder: &mut impl Source<Item = Sample>,
    position: Duration,
    cancelled: &AtomicBool,
) {
    // Round to whole frames before counting interleaved samples; rounding
    // after multiplying by channels could swap left/right at fractional frames.
    let frames = position
        .as_nanos()
        .saturating_mul(decoder.sample_rate() as u128)
        / 1_000_000_000;
    let samples = frames.saturating_mul(decoder.channels() as u128);
    for _ in 0..samples {
        if cancelled.load(Ordering::Acquire) {
            return;
        }
        if decoder.next().is_none() {
            break; // Seeking to the end is ordinary completion, not a playback failure.
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
type StereoConsumer = HeapCons<(f32, f32)>;

#[derive(Clone, Default)]
pub struct SampleTap {
    consumer: Arc<Mutex<Option<StereoConsumer>>>,
}

impl std::fmt::Debug for SampleTap {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SampleTap(..)")
    }
}

impl SampleTap {
    #[cfg(test)]
    pub(crate) fn from_test_samples(samples: &[(f32, f32)]) -> Self {
        let (mut producer, consumer) = HeapRb::new(samples.len().max(1)).split();
        for sample in samples {
            producer.try_push(*sample).expect("fixture capacity");
        }
        let tap = Self::default();
        tap.install(consumer);
        tap
    }

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

enum PcmFrame {
    Start,
    Data(usize),
    Silence(usize),
}

/// Nonblocking source read by rodio's mixer callback.
struct PcmSource {
    consumer: HeapCons<Sample>,
    frame: PcmFrame,
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
        if matches!(self.frame, PcmFrame::Start) {
            if self.cancelled.load(Ordering::Acquire) {
                return None;
            }
            let channels = usize::from(self.channels).max(1);
            let finished = self.finished.load(Ordering::Acquire);
            self.frame = if self.consumer.occupied_len() >= channels {
                PcmFrame::Data(channels)
            } else if finished {
                return None;
            } else {
                PcmFrame::Silence(channels)
            };
        }
        let (sample, remaining) = match &mut self.frame {
            PcmFrame::Data(remaining) => (self.consumer.try_pop()?, remaining),
            PcmFrame::Silence(remaining) => {
                self.underruns.fetch_add(1, Ordering::Relaxed);
                (0., remaining)
            }
            PcmFrame::Start => unreachable!(),
        };
        *remaining -= 1;
        if *remaining == 0 {
            self.frame = PcmFrame::Start;
        }
        Some(sample)
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
    current_data: Option<BufferedInput>,
    base_position: Duration,
    underruns: Arc<AtomicU64>,
    pipeline_tx: std_mpsc::SyncSender<PipelineMessage>,
    pipeline_rx: std_mpsc::Receiver<PipelineMessage>,
    active_playback_id: Option<u64>,
    fading: Option<FadingTrack>,
}

struct FadingTrack {
    sink: Sink,
    cancelled: Option<Arc<AtomicBool>>,
    decoder: Option<JoinHandle<()>>,
    timing: FadeTiming,
}
enum FadeTiming {
    Waiting {
        remaining: Duration,
        outgoing_start: Duration,
    },
    Mixing {
        duration: Duration,
    },
}
impl FadeTiming {
    fn sample(&mut self, outgoing: Duration, incoming: Option<Duration>) -> (f32, f32, bool) {
        if let Self::Waiting {
            remaining,
            outgoing_start,
        } = self
            && incoming.is_some()
        {
            let elapsed = outgoing.saturating_sub(*outgoing_start);
            *self = Self::Mixing {
                duration: remaining
                    .saturating_sub(elapsed)
                    .max(Duration::from_millis(20)),
            };
        }
        match self {
            Self::Waiting { .. } => (1., 0., false),
            Self::Mixing { duration } => {
                let position = incoming.unwrap_or_default();
                let (outgoing, incoming) = transition_gains(position, *duration);
                (outgoing, incoming, position >= *duration)
            }
        }
    }
}
impl Drop for FadingTrack {
    fn drop(&mut self) {
        self.sink.stop();
        if let Some(cancelled) = &self.cancelled {
            cancelled.store(true, Ordering::Release);
        }
        if let Some(decoder) = self.decoder.take() {
            super::finish_thread(decoder, "transition decoder", Duration::from_millis(250));
        }
    }
}

fn transition_gains(position: Duration, duration: Duration) -> (f32, f32) {
    let fraction = (position.as_secs_f32() / duration.as_secs_f32().max(0.001)).clamp(0., 1.);
    let angle = fraction * std::f32::consts::FRAC_PI_2;
    (angle.cos().max(0.), angle.sin())
}

#[cfg(test)]
mod transition_tests {
    use super::*;
    #[test]
    fn overlap_waits_for_audio_then_adapts_to_buffering_and_pause() {
        let mut timing = FadeTiming::Waiting {
            remaining: Duration::from_secs(4),
            outgoing_start: Duration::from_secs(56),
        };
        assert_eq!(
            timing.sample(Duration::from_millis(57_500), None),
            (1., 0., false)
        );
        assert_eq!(
            timing.sample(Duration::from_millis(57_500), Some(Duration::ZERO)),
            (1., 0., false)
        );
        let gains = timing.sample(
            Duration::from_millis(58_750),
            Some(Duration::from_millis(1250)),
        );
        assert!((gains.0 - gains.1).abs() < 0.0001 && !gains.2);
        assert_eq!(
            timing.sample(
                Duration::from_millis(58_750),
                Some(Duration::from_millis(1250))
            ),
            gains
        );
        assert!(
            timing
                .sample(Duration::from_secs(60), Some(Duration::from_millis(2500)))
                .2
        );
    }
    #[test]
    fn overlap_preserves_power_and_reaches_silent_endpoints() {
        let duration = Duration::from_secs(4);
        assert_eq!(transition_gains(Duration::ZERO, duration), (1., 0.));
        let mut previous = (1., 0.);
        for millis in (0..=4000).step_by(20) {
            let gains = transition_gains(Duration::from_millis(millis), duration);
            assert!((gains.0 * gains.0 + gains.1 * gains.1 - 1.).abs() < 0.0001);
            assert!(gains.0 <= previous.0 && gains.1 >= previous.1);
            previous = gains;
        }
        assert!(previous.0 < 0.0001 && previous.1 == 1.);
        assert_eq!(transition_gains(Duration::from_secs(8), duration), previous);
    }
}

impl RodioBackend {
    /// Open the system's default audio output.
    ///
    /// # Errors
    /// Returns an error when the operating system cannot open its default device.
    pub fn new() -> Result<Self, AudioError> {
        let stream = OutputStreamBuilder::open_default_stream()
            .map_err(|error| AudioError::NoDevice(error.to_string()))?;
        Ok(Self::from_stream(stream))
    }

    fn from_stream(mut stream: OutputStream) -> Self {
        stream.log_on_drop(false);

        // A decoder emits at most Ready plus Failed. Keeping this bounded
        // prevents a detached or buggy worker from growing actor memory.
        let (pipeline_tx, pipeline_rx) = std_mpsc::sync_channel(2);
        Self {
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
            fading: None,
        }
    }

    pub fn sample_tap(&self) -> SampleTap {
        self.tap.clone()
    }

    pub(crate) fn try_decode(
        data: impl Into<BufferedInput>,
    ) -> Result<Decoder<Box<dyn AudioReader>>, AudioError> {
        let (reader, byte_len): (Box<dyn AudioReader>, u64) = match data.into() {
            BufferedInput::Bytes(data) => {
                let len = data.len() as u64;
                (Box::new(Cursor::new(SharedBytes(data))), len)
            }
            BufferedInput::File(file) => {
                let input = std::fs::File::open(&file.path)
                    .map_err(|e| AudioError::DecodeError(e.to_string()))?;
                let len = input
                    .metadata()
                    .map_err(|e| AudioError::DecodeError(e.to_string()))?
                    .len();
                (Box::new(std::io::BufReader::new(input)), len)
            }
        };
        match crate::util::catch_expected_panic(move || {
            Decoder::builder()
                .with_data(reader)
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

    pub(crate) fn try_decode_stream(
        input: StreamingInput,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Decoder<super::spool::Reader>, AudioError> {
        let reader = input
            .spool
            .reader(cancelled)
            .map_err(|error| AudioError::DecodeError(error.to_string()))?;
        let mime_type = input.mime_type;
        match crate::util::catch_expected_panic(move || {
            let builder = Decoder::builder().with_data(reader).with_seekable(false);
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
            super::finish_thread(handle, "audio decoder", Duration::from_millis(250));
        }
        self.tap.clear();
        self.active_playback_id = None;
        while self.pipeline_rx.try_recv().is_ok() {}
    }

    pub(crate) fn begin_transition(&mut self, duration: Duration) {
        self.fading = self.sink.take().map(|sink| FadingTrack {
            timing: FadeTiming::Waiting {
                remaining: duration,
                outgoing_start: sink.get_pos(),
            },
            sink,
            cancelled: self.decoder_cancelled.take(),
            decoder: self.decoder_thread.take(),
        });
        self.active_playback_id = None;
    }

    pub(crate) fn start_buffered(
        &mut self,
        data: BufferedInput,
        start_position: Duration,
    ) -> Result<(), AudioError> {
        let paused = self.is_paused();
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
        let prebuffer_samples = samples_per_second.saturating_mul(PCM_PREBUFFER_MS) / 1_000;

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

        // Keep the old playback intact until decoding/seeking has succeeded.
        self.stop_pipeline();
        let finished = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_finished = finished.clone();
        let worker_cancelled = cancelled.clone();
        let decoder_thread = std::thread::Builder::new()
            .name("textamp-audio-decoder".to_string())
            .spawn(move || {
                let _finished_guard = DecoderFinished(worker_finished);
                let _ = crate::util::catch_expected_panic(move || {
                    fill_pcm(&mut decoder, &mut producer, &worker_cancelled);
                });
            })
            .map_err(|error| AudioError::PlaybackError(error.to_string()))?;

        let source = PcmSource {
            consumer,
            frame: PcmFrame::Start,
            finished,
            cancelled: cancelled.clone(),
            underruns: self.underruns.clone(),
            channels,
            sample_rate,
            duration,
        };
        let tapped = TapSource::new(source, &self.tap);
        let sink = Sink::connect_new(self.stream.mixer());
        sink.set_volume(if self.fading.is_some() {
            0.
        } else {
            self.volume
        });
        if paused {
            sink.pause();
        }
        sink.append(tapped);

        self.sink = Some(sink);
        self.decoder_cancelled = Some(cancelled);
        self.decoder_thread = Some(decoder_thread);
        self.current_data = Some(data);
        self.base_position = start_position;
        Ok(())
    }

    pub(crate) fn seek_buffered(
        &mut self,
        data: BufferedInput,
        position: Duration,
    ) -> Result<(), AudioError> {
        self.fading = None;
        self.start_buffered(data, position)
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
        let start_position = input.start_position;

        let decoder_thread = std::thread::Builder::new()
            .name("textamp-stream-decoder".to_string())
            .spawn(move || {
                let run_cancelled = worker_cancelled.clone();
                let run_failure = failure.clone();
                let ready_tx = pipeline_tx.clone();
                let result =
                    crate::util::catch_expected_panic(move || -> Result<(), AudioError> {
                        let mut decoder = Self::try_decode_stream(input, run_cancelled.clone())?;
                        skip_stream_to(&mut decoder, start_position, &run_cancelled);
                        if run_cancelled.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        let channels = decoder.channels();
                        let sample_rate = decoder.sample_rate();
                        let samples_per_second = sample_rate as usize * channels as usize;
                        let capacity = samples_per_second
                            .saturating_mul(PCM_BUFFER_SECONDS)
                            .max(8_192);
                        let prebuffer_samples =
                            samples_per_second.saturating_mul(PCM_PREBUFFER_MS) / 1_000;
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
                            frame: PcmFrame::Start,
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
                        // The actor applies the latest pause state before starting
                        // output, including a seek performed while paused.
                        sink.pause();
                        sink.append(tapped);
                        if ready_tx
                            .send(PipelineMessage::Ready { playback_id, sink })
                            .is_err()
                        {
                            return Ok(());
                        }

                        fill_pcm(&mut decoder, &mut producer, &run_cancelled);
                        if let Some(message) = run_failure.message() {
                            return Err(AudioError::PlaybackError(message));
                        }
                        Ok(())
                    });

                let message = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error.to_string()),
                    Err(_) => Some("decoder crashed while streaming media".to_string()),
                };
                if !worker_cancelled.load(Ordering::Acquire)
                    && let Some(message) = message
                {
                    let _ = pipeline_tx.send(PipelineMessage::Failed {
                        playback_id,
                        message,
                    });
                }
            })
            .map_err(|error| AudioError::PlaybackError(error.to_string()))?;

        self.decoder_cancelled = Some(cancelled);
        self.decoder_thread = Some(decoder_thread);
        self.current_data = None;
        self.base_position = start_position;
        self.active_playback_id = Some(playback_id);
        Ok(())
    }

    fn update_transition(&mut self) {
        if let Some(fading) = &mut self.fading {
            let (outgoing, incoming, complete) = fading
                .timing
                .sample(fading.sink.get_pos(), self.sink.as_ref().map(Sink::get_pos));
            fading.sink.set_volume(self.volume * outgoing);
            if let Some(sink) = &self.sink {
                sink.set_volume(self.volume * incoming);
            }
            if complete || fading.sink.empty() {
                self.fading = None;
                if let Some(sink) = &self.sink {
                    sink.set_volume(self.volume);
                }
            }
        }
    }

    pub(crate) fn poll_pipeline(&mut self, paused: bool) -> Vec<PipelineEvent> {
        if let Some(fading) = &self.fading {
            if paused {
                fading.sink.pause();
            } else {
                fading.sink.play();
            }
        }
        self.update_transition();
        let mut events = Vec::new();
        while let Ok(message) = self.pipeline_rx.try_recv() {
            match message {
                PipelineMessage::Ready { playback_id, sink }
                    if self.active_playback_id == Some(playback_id) =>
                {
                    if self.fading.is_some() {
                        sink.set_volume(0.);
                    }
                    if !paused {
                        sink.play();
                    }
                    self.sink = Some(sink);
                    events.push(PipelineEvent::Ready { playback_id });
                }
                PipelineMessage::Ready { sink, .. } => sink.stop(),
                PipelineMessage::Failed {
                    playback_id,
                    message,
                } if self.active_playback_id == Some(playback_id) => {
                    self.fading = None;
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
        self.start_buffered(data.into(), Duration::ZERO)
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
        self.fading = None;
        self.stop_pipeline();
        self.current_data = None;
        self.base_position = Duration::ZERO;
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(sink) = &self.sink {
            sink.set_volume(self.volume);
        }
        self.update_transition();
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
        self.fading = None;
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
    fn pcm_underruns_preserve_channel_alignment_and_drain_completed_frames() {
        for channels in [1, 2, 6] {
            let (mut producer, consumer) = HeapRb::new(64).split();
            let finished = Arc::new(AtomicBool::new(false));
            let underruns = Arc::new(AtomicU64::new(0));
            let mut source = PcmSource {
                consumer,
                frame: PcmFrame::Start,
                finished: finished.clone(),
                cancelled: Arc::new(AtomicBool::new(false)),
                underruns: underruns.clone(),
                channels,
                sample_rate: 48_000,
                duration: None,
            };
            assert_eq!(source.next(), Some(0.));
            let frame: Vec<f32> = (1..=channels).map(f32::from).collect();
            producer.push_slice(&frame);
            for _ in 1..channels {
                assert_eq!(source.next(), Some(0.));
            }
            for sample in &frame {
                assert_eq!(source.next(), Some(*sample));
            }
            assert_eq!(underruns.load(Ordering::Relaxed), u64::from(channels));
            if channels > 1 {
                producer.try_push(10.).unwrap();
                assert_eq!(source.next(), Some(0.));
                producer.push_slice(&frame[1..]);
                for _ in 1..channels {
                    assert_eq!(source.next(), Some(0.));
                }
                assert_eq!(source.next(), Some(10.));
                for sample in &frame[1..] {
                    assert_eq!(source.next(), Some(*sample));
                }
            }
            producer.push_slice(&frame);
            finished.store(true, Ordering::Release);
            for sample in &frame {
                assert_eq!(source.next(), Some(*sample));
            }
            assert_eq!(source.next(), None);
        }
    }

    #[test]
    fn block_decoder_preserves_samples_under_bounded_backpressure() {
        let expected: Vec<_> = (0..8192).map(|value| value as f32).collect();
        let (mut producer, mut consumer) = HeapRb::new(32).split();
        let cancelled = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let input = expected.clone();
            let worker =
                scope.spawn(|| fill_pcm(&mut input.into_iter(), &mut producer, &cancelled));
            let mut actual = Vec::new();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while actual.len() < expected.len() {
                assert!(std::time::Instant::now() < deadline, "PCM producer stalled");
                if let Some(sample) = consumer.try_pop() {
                    actual.push(sample);
                } else {
                    std::thread::yield_now();
                }
            }
            worker.join().unwrap();
            assert_eq!(actual, expected);
        });
    }

    fn seek_wav() -> Vec<u8> {
        let data_len = 4_000 * 2 * 2;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&1_000u32.to_le_bytes());
        wav.extend_from_slice(&4_000u32.to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data_len as u32).to_le_bytes());
        for frame in 0..4_000i16 {
            for _ in 0..2 {
                wav.extend_from_slice(&(frame * 4).to_le_bytes());
            }
        }
        wav
    }

    #[test]
    fn temporary_file_seeks_to_requested_frames_without_audio_hardware() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&seek_wav()).unwrap();
        let input = BufferedInput::File(crate::library::MediaFile::temporary(file));
        let mut decoder = RodioBackend::try_decode(input).unwrap();
        for millis in [500, 3000, 1000] {
            decoder.try_seek(Duration::from_millis(millis)).unwrap();
            let expected = (millis * 4) as f32 / 32768.;
            assert!((decoder.next().unwrap() - expected).abs() < 0.0001);
            assert!((decoder.next().unwrap() - expected).abs() < 0.0001);
        }
    }

    #[test]
    fn streamed_seek_decodes_to_the_requested_stereo_frame() {
        for millis in [0, 500, 1_500, 3_000] {
            let bytes = seek_wav();
            let (spool, _) = super::super::spool::Spool::create().unwrap();
            std::fs::write(&spool.file().path, &bytes).unwrap();
            spool.published(bytes.len());
            spool.finish(None);
            let cancelled = Arc::new(AtomicBool::new(false));
            let input = StreamingInput::from_spool(
                spool,
                StreamFailure::default(),
                Some("audio/wav".into()),
                Duration::ZERO,
            );
            let mut decoder = RodioBackend::try_decode_stream(input, cancelled.clone()).unwrap();
            skip_stream_to(&mut decoder, Duration::from_millis(millis), &cancelled);
            let expected = (millis * 4) as f32 / 32768.0;
            for _ in 0..2 {
                assert!((decoder.next().unwrap() - expected).abs() < 0.0001);
            }
        }
    }

    #[test]
    fn streamed_seek_preserves_channels_at_fractional_frame_offsets() {
        let samples: Vec<_> = (0..1_000)
            .flat_map(|frame| [frame as f32, -(frame as f32)])
            .collect();
        let mut decoder = rodio::buffer::SamplesBuffer::new(2, 44_100, samples);
        skip_stream_to(
            &mut decoder,
            Duration::from_millis(5),
            &AtomicBool::new(false),
        );
        assert_eq!(decoder.next(), Some(220.0));
        assert_eq!(decoder.next(), Some(-220.0));
    }

    #[test]
    fn streamed_seek_cancels_and_end_of_track_is_not_an_error() {
        let mut decoder = rodio::buffer::SamplesBuffer::new(2, 1_000, vec![0.25; 2_000]);
        skip_stream_to(
            &mut decoder,
            Duration::from_secs(10),
            &AtomicBool::new(true),
        );
        assert_eq!(decoder.next(), Some(0.25)); // Cancelled before consuming samples.
        skip_stream_to(
            &mut decoder,
            Duration::from_secs(10),
            &AtomicBool::new(false),
        );
        assert_eq!(decoder.next(), None);
    }

    #[test]
    fn progressive_reader_cancellation_does_not_wait_for_network() {
        let (spool, _) = super::super::spool::Spool::create().unwrap();
        let input =
            StreamingInput::from_spool(spool, StreamFailure::default(), None, Duration::ZERO);
        let mut reader = input.spool.reader(Arc::new(AtomicBool::new(true))).unwrap();
        assert_eq!(reader.read(&mut [0; 8]).unwrap(), 0);
    }
    #[test]
    fn progressive_reader_propagates_transfer_failure() {
        let (spool, _) = super::super::spool::Spool::create().unwrap();
        spool.finish(Some("network interrupted".into()));
        let input =
            StreamingInput::from_spool(spool, StreamFailure::default(), None, Duration::ZERO);
        let mut reader = input
            .spool
            .reader(Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert_eq!(
            reader.read(&mut [0; 8]).unwrap_err().to_string(),
            "network interrupted"
        );
    }
}
