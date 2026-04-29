//! Rodio-based audio backend implementation.
//!
//! This is the default audio backend for the TUI application,
//! using the rodio library with symphonia for decoding.

use super::streaming::BlockingReader;
use super::traits::{AudioBackend, AudioError};
use rodio::source::Source;
use rodio::{cpal::traits::{DeviceTrait, HostTrait}, ChannelCount, Decoder, OutputStream, OutputStreamBuilder, Sample, SampleRate, Sink};
use std::collections::VecDeque;
use std::io::Cursor;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Shared, thread-safe ring of recent stereo samples that the audio
/// pipeline keeps populating while playback runs. Used by the
/// vectorscope visualizer (drained on each UI tick).
///
/// Each entry is `(left, right)` in `[-1.0, 1.0]`. We pre-cap the
/// buffer at `SAMPLE_TAP_CAP` so a stalled UI never lets the audio
/// thread fill memory unboundedly.
pub type SampleTap = Arc<Mutex<VecDeque<(f32, f32)>>>;

/// ~85 ms of stereo samples at 48 kHz. Long enough to survive a
/// dropped or jittery UI frame; short enough that a stale tap can't
/// inflate a burst of historic samples once playback resumes.
const SAMPLE_TAP_CAP: usize = 4_096;

fn new_sample_tap() -> SampleTap {
    Arc::new(Mutex::new(VecDeque::with_capacity(SAMPLE_TAP_CAP)))
}

/// Source wrapper that copies every sample it forwards into a shared
/// `SampleTap`. Stereo sources are split into `(L, R)` pairs at the
/// interleave boundary; mono sources duplicate the sample to both
/// channels so the vectorscope still has something to plot (the
/// trace becomes a diagonal line, which is the correct mono visual).
struct TapSource<S> {
    inner: S,
    tap: SampleTap,
    pending_left: Option<f32>,
    is_stereo: bool,
}

impl<S> TapSource<S>
where
    S: Source<Item = Sample>,
{
    fn new(inner: S, tap: SampleTap) -> Self {
        let is_stereo = inner.channels() >= 2;
        Self { inner, tap, pending_left: None, is_stereo }
    }

    #[inline]
    fn push_pair(&self, pair: (f32, f32)) {
        if let Ok(mut buf) = self.tap.lock() {
            if buf.len() == SAMPLE_TAP_CAP {
                buf.pop_front();
            }
            buf.push_back(pair);
        }
    }
}

impl<S> Iterator for TapSource<S>
where
    S: Source<Item = Sample>,
{
    type Item = Sample;

    #[inline]
    fn next(&mut self) -> Option<Sample> {
        let s = self.inner.next()?;
        if self.is_stereo {
            match self.pending_left.take() {
                Some(l) => self.push_pair((l, s)),
                None => self.pending_left = Some(s),
            }
        } else {
            self.push_pair((s, s));
        }
        Some(s)
    }
}

impl<S> Source for TapSource<S>
where
    S: Source<Item = Sample>,
{
    fn current_span_len(&self) -> Option<usize> { self.inner.current_span_len() }
    fn channels(&self) -> ChannelCount { self.inner.channels() }
    fn sample_rate(&self) -> SampleRate { self.inner.sample_rate() }
    fn total_duration(&self) -> Option<Duration> { self.inner.total_duration() }

    // Forward seek to the inner source — without this the default impl
    // returns `SeekError::NotSupported` and Sink::try_seek silently
    // fails, breaking click-to-seek on the waveform / spectrogram /
    // transport-bar slider.
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        // Drop any half-collected pending sample — the inner cursor
        // is moving so the (L, R) split would otherwise pair a left
        // sample from the old position with a right sample from the
        // new one.
        self.pending_left = None;
        // Same reason the buffer is cleared on play_data: don't keep
        // pre-seek tail samples mixed with post-seek ones.
        if let Ok(mut buf) = self.tap.lock() { buf.clear(); }
        self.inner.try_seek(pos)
    }
}

/// Wrapper around `Arc<Vec<u8>>` that implements `AsRef<[u8]>`,
/// enabling zero-copy use with `Cursor` for rodio decoding.
#[derive(Clone)]
struct SharedBytes(Arc<Vec<u8>>);

impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Audio backend using rodio for playback.
///
/// Supports most common audio formats via symphonia:
/// - MP3, FLAC, AAC, ALAC, WAV, OGG, etc.
pub struct RodioBackend {
    stream: OutputStream,
    sink: Option<Sink>,
    volume: f32,
    tap: SampleTap,
}

impl RodioBackend {
    /// Create a new rodio-based audio backend.
    ///
    /// First tries the system default output device. If Windows has
    /// no default configured (HRESULT 0x80070490 "Element not found"
    /// from `GetDefaultAudioEndpoint` — common after a Bluetooth or
    /// USB output is disconnected), walks the host's available output
    /// devices and opens the first one that accepts a stream. This
    /// keeps playback working when the user still has usable outputs,
    /// just not a preferred default.
    pub fn new() -> Result<Self, AudioError> {
        let mut stream = match OutputStreamBuilder::open_default_stream() {
            Ok(s) => s,
            Err(default_err) => {
                // Walk every available output endpoint and try each one.
                // Uses `open_stream_or_fallback` so rodio will cycle
                // through supported configs (RDP virtual sinks and
                // some USB DACs reject the default sample-rate/channel
                // combo and only accept a specific one).
                let host = rodio::cpal::default_host();
                tracing::error!("audio: default stream failed: {}", default_err);
                let mut last_err = default_err.to_string();
                let mut opened: Option<OutputStream> = None;
                let devices = host.output_devices().map_err(|e| {
                    AudioError::NoDevice(format!("host.output_devices() failed: {e}"))
                })?;
                let mut count = 0usize;
                for dev in devices {
                    count += 1;
                    let name = dev.name().unwrap_or_else(|_| "<unnamed>".to_string());
                    match OutputStreamBuilder::from_device(dev) {
                        Ok(builder) => match builder.open_stream_or_fallback() {
                            Ok(s) => {
                                tracing::error!("audio: opened fallback output device '{}'", name);
                                opened = Some(s);
                                break;
                            }
                            Err(e) => {
                                tracing::error!("audio: '{}' open_stream_or_fallback failed: {}", name, e);
                                last_err = format!("{name}: {e}");
                            }
                        },
                        Err(e) => {
                            tracing::error!("audio: '{}' from_device failed: {}", name, e);
                            last_err = format!("{name}: {e}");
                        }
                    }
                }
                if count == 0 {
                    tracing::error!("audio: host enumerated zero output devices");
                }
                match opened {
                    Some(s) => s,
                    None => return Err(AudioError::NoDevice(last_err)),
                }
            }
        };

        // Don't log messages on drop that could corrupt the TUI
        stream.log_on_drop(false);

        Ok(Self {
            stream,
            sink: None,
            volume: 0.8,
            tap: new_sample_tap(),
        })
    }

    /// Clone the shared sample buffer so other layers (the GUI
    /// vectorscope visualizer) can drain stereo samples in parallel
    /// with playback.
    pub fn sample_tap(&self) -> SampleTap {
        self.tap.clone()
    }

    /// Try to decode audio data, catching any panics silently.
    ///
    /// Some audio files can cause symphonia to panic due to bugs
    /// in the decoder. This method catches those panics and returns
    /// a proper error instead.
    fn try_decode(data: Arc<Vec<u8>>) -> Result<Decoder<Cursor<SharedBytes>>, AudioError> {
        let prev_hook = panic::take_hook();
        panic::set_hook(Box::new(|_| {
            // Silently ignore panics during decode
        }));

        let byte_len = data.len() as u64;

        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let cursor = Cursor::new(SharedBytes(data));
            Decoder::builder()
                .with_data(cursor)
                .with_byte_len(byte_len)
                .build()
        }));

        panic::set_hook(prev_hook);

        match result {
            Ok(Ok(source)) => Ok(source),
            Ok(Err(e)) => Err(AudioError::DecodeError(e.to_string())),
            Err(_) => Err(AudioError::DecodeError(
                "Decoder crashed - unsupported format".to_string(),
            )),
        }
    }

    /// Try to decode from a streaming reader, catching any panics.
    fn try_decode_streaming(reader: BlockingReader, byte_len: u64) -> Result<Decoder<BlockingReader>, AudioError> {
        let prev_hook = panic::take_hook();
        panic::set_hook(Box::new(|_| {
            // Silently ignore panics during decode
        }));

        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            Decoder::builder()
                .with_data(reader)
                .with_byte_len(byte_len)
                .build()
        }));

        panic::set_hook(prev_hook);

        match result {
            Ok(Ok(source)) => Ok(source),
            Ok(Err(e)) => Err(AudioError::DecodeError(e.to_string())),
            Err(_) => Err(AudioError::DecodeError(
                "Decoder crashed - unsupported format".to_string(),
            )),
        }
    }

    /// Play audio from a streaming buffer.
    ///
    /// This allows playback to start before the full file is downloaded.
    pub fn play_streaming(&mut self, reader: BlockingReader, byte_len: u64) -> Result<(), AudioError> {
        // Stop any existing playback first
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }

        // Decode from the streaming reader
        let source = Self::try_decode_streaming(reader, byte_len)?;
        // Drain the previous track's tail samples so the vectorscope
        // doesn't briefly draw the old waveform when a new track starts.
        if let Ok(mut buf) = self.tap.lock() { buf.clear(); }
        let tapped = TapSource::new(source, self.tap.clone());

        // Create new sink and start playback
        let sink = Sink::connect_new(self.stream.mixer());
        sink.set_volume(self.volume);
        sink.append(tapped);

        self.sink = Some(sink);

        Ok(())
    }
}

impl AudioBackend for RodioBackend {
    fn play_data(&mut self, data: Arc<Vec<u8>>) -> Result<(), AudioError> {
        // Stop any existing playback first
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }

        // Decode the audio data
        let source = Self::try_decode(data)?;
        if let Ok(mut buf) = self.tap.lock() { buf.clear(); }
        let tapped = TapSource::new(source, self.tap.clone());

        // Create new sink and start playback
        let sink = Sink::connect_new(self.stream.mixer());
        sink.set_volume(self.volume);
        sink.append(tapped);

        self.sink = Some(sink);

        Ok(())
    }

    fn pause(&mut self) {
        if let Some(ref sink) = self.sink {
            sink.pause();
        }
    }

    fn resume(&mut self) {
        if let Some(ref sink) = self.sink {
            sink.play();
        }
    }

    fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(ref sink) = self.sink {
            sink.set_volume(self.volume);
        }
    }

    fn volume(&self) -> f32 {
        self.volume
    }

    fn is_finished(&self) -> bool {
        match &self.sink {
            Some(sink) => sink.empty(),
            // No sink = nothing was playing, not "finished".
            // Returning false prevents spurious TrackEnded when status is
            // incorrectly set to Playing without actual audio output.
            None => false,
        }
    }

    fn is_playing(&self) -> bool {
        match &self.sink {
            Some(sink) => !sink.is_paused() && !sink.empty(),
            None => false,
        }
    }

    fn is_paused(&self) -> bool {
        match &self.sink {
            Some(sink) => sink.is_paused(),
            None => false,
        }
    }

    fn seek(&mut self, position: Duration) -> bool {
        if let Some(ref sink) = self.sink {
            if let Err(e) = sink.try_seek(position) {
                tracing::warn!("Seek failed: {}", e);
                return false;
            }
            return true;
        }
        false
    }

    fn position(&self) -> Option<Duration> {
        self.sink.as_ref().map(|sink| sink.get_pos())
    }
}
