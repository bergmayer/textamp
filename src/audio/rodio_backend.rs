//! Rodio-based audio backend implementation.
//!
//! This is the default audio backend for the TUI application,
//! using the rodio library with symphonia for decoding.

use super::streaming::BlockingReader;
use super::traits::{AudioBackend, AudioError};
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink};
use std::io::Cursor;
use std::panic::{self, AssertUnwindSafe};
use std::time::Duration;

/// Audio backend using rodio for playback.
///
/// Supports most common audio formats via symphonia:
/// - MP3, FLAC, AAC, ALAC, WAV, OGG, etc.
pub struct RodioBackend {
    stream: OutputStream,
    sink: Option<Sink>,
    volume: f32,
}

impl RodioBackend {
    /// Create a new rodio-based audio backend.
    ///
    /// Returns an error if no audio device is available.
    pub fn new() -> Result<Self, AudioError> {
        let mut stream = OutputStreamBuilder::open_default_stream()
            .map_err(|_| AudioError::NoDevice)?;

        // Don't log messages on drop that could corrupt the TUI
        stream.log_on_drop(false);

        Ok(Self {
            stream,
            sink: None,
            volume: 0.8,
        })
    }

    /// Try to decode audio data, catching any panics silently.
    ///
    /// Some audio files can cause symphonia to panic due to bugs
    /// in the decoder. This method catches those panics and returns
    /// a proper error instead.
    fn try_decode(data: Vec<u8>) -> Result<Decoder<Cursor<Vec<u8>>>, AudioError> {
        let prev_hook = panic::take_hook();
        panic::set_hook(Box::new(|_| {
            // Silently ignore panics during decode
        }));

        let byte_len = data.len() as u64;

        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let cursor = Cursor::new(data);
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

        // Create new sink and start playback
        let sink = Sink::connect_new(self.stream.mixer());
        sink.set_volume(self.volume);
        sink.append(source);

        self.sink = Some(sink);

        Ok(())
    }
}

impl AudioBackend for RodioBackend {
    fn play_data(&mut self, data: Vec<u8>) -> Result<(), AudioError> {
        // Stop any existing playback first
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }

        // Decode the audio data
        let source = Self::try_decode(data)?;

        // Create new sink and start playback
        let sink = Sink::connect_new(self.stream.mixer());
        sink.set_volume(self.volume);
        sink.append(source);

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
            None => true,
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
