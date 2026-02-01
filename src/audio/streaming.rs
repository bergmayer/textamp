//! Streaming audio buffer for progressive download playback.
//!
//! This module provides a buffer that allows playback to start
//! before the entire file is downloaded, significantly reducing
//! time-to-first-audio.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

/// Minimum buffer size before playback can start (512KB).
/// Larger buffer reduces blocking waits during decoder format detection.
const MIN_BUFFER_SIZE: usize = 512 * 1024;

/// Streaming buffer that grows as data arrives.
///
/// Implements `Read + Seek` for compatibility with rodio's Decoder.
/// Seeking beyond buffered data will block until data arrives.
#[derive(Clone)]
pub struct StreamingBuffer {
    inner: Arc<Mutex<BufferState>>,
}

struct BufferState {
    /// Downloaded data
    data: Vec<u8>,
    /// Total expected size (from Content-Length)
    total_size: Option<usize>,
    /// Download complete flag
    complete: bool,
    /// Error that occurred during download
    error: Option<String>,
}

impl StreamingBuffer {
    /// Create a new empty streaming buffer.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BufferState {
                data: Vec::new(),
                total_size: None,
                complete: false,
                error: None,
            })),
        }
    }

    /// Create a new streaming buffer with known total size.
    pub fn with_size_hint(total_size: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BufferState {
                data: Vec::with_capacity(total_size),
                total_size: Some(total_size),
                complete: false,
                error: None,
            })),
        }
    }

    /// Append data to the buffer.
    ///
    /// Called by the download task as chunks arrive.
    pub fn append(&self, chunk: &[u8]) {
        let mut state = self.inner.lock().unwrap();
        state.data.extend_from_slice(chunk);
    }

    /// Mark the download as complete.
    pub fn set_complete(&self) {
        let mut state = self.inner.lock().unwrap();
        state.complete = true;
    }

    /// Mark an error occurred during download.
    pub fn set_error(&self, error: String) {
        let mut state = self.inner.lock().unwrap();
        state.error = Some(error);
        state.complete = true;
    }

    /// Check if minimum buffer for playback is available.
    pub fn has_min_buffer(&self) -> bool {
        let state = self.inner.lock().unwrap();
        state.data.len() >= MIN_BUFFER_SIZE || state.complete
    }

    /// Get current buffered size.
    pub fn buffered_size(&self) -> usize {
        self.inner.lock().unwrap().data.len()
    }

    /// Check if download is complete.
    pub fn is_complete(&self) -> bool {
        self.inner.lock().unwrap().complete
    }

    /// Get download error if any.
    pub fn error(&self) -> Option<String> {
        self.inner.lock().unwrap().error.clone()
    }

    /// Get download progress as a fraction (0.0 to 1.0).
    pub fn progress(&self) -> f64 {
        let state = self.inner.lock().unwrap();
        match state.total_size {
            Some(total) if total > 0 => state.data.len() as f64 / total as f64,
            _ if state.complete => 1.0,
            _ => 0.0,
        }
    }

    /// Create a reader that can be used for decoding.
    ///
    /// The reader shares the same underlying buffer.
    pub fn reader(&self) -> BufferReader {
        BufferReader {
            buffer: self.inner.clone(),
            position: 0,
        }
    }

    /// Get total data as bytes (for compatibility with existing code).
    pub fn into_bytes(self) -> Vec<u8> {
        let state = self.inner.lock().unwrap();
        state.data.clone()
    }
}

impl Default for StreamingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Reader view into a StreamingBuffer.
///
/// Multiple readers can exist for the same buffer, each with
/// their own read position.
pub struct BufferReader {
    buffer: Arc<Mutex<BufferState>>,
    position: usize,
}

impl Read for BufferReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let state = self.buffer.lock().unwrap();

        // Check for download error
        if let Some(ref err) = state.error {
            return Err(io::Error::new(io::ErrorKind::Other, err.clone()));
        }

        // If we're at or past the buffered data
        if self.position >= state.data.len() {
            if state.complete {
                // EOF
                return Ok(0);
            } else {
                // Data not yet available - return WouldBlock
                // The caller should retry after more data arrives
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Waiting for more data",
                ));
            }
        }

        // Read available data
        let available = &state.data[self.position..];
        let to_read = buf.len().min(available.len());
        buf[..to_read].copy_from_slice(&available[..to_read]);
        self.position += to_read;

        Ok(to_read)
    }
}

impl Seek for BufferReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let state = self.buffer.lock().unwrap();

        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::Current(offset) => self.position as i64 + offset,
            SeekFrom::End(offset) => {
                // For SeekFrom::End, we need to know the total size
                if let Some(total) = state.total_size {
                    total as i64 + offset
                } else if state.complete {
                    state.data.len() as i64 + offset
                } else {
                    // Can't seek from end if size unknown and incomplete
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "Cannot seek from end: size unknown",
                    ));
                }
            }
        };

        if new_pos < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Seek to negative position",
            ));
        }

        self.position = new_pos as usize;
        Ok(self.position as u64)
    }
}

/// Blocking reader that waits for data instead of returning WouldBlock.
///
/// Use this when you need blocking behavior (e.g., for rodio).
pub struct BlockingReader {
    buffer: Arc<Mutex<BufferState>>,
    position: usize,
}

impl BlockingReader {
    pub fn new(buffer: &StreamingBuffer) -> Self {
        Self {
            buffer: buffer.inner.clone(),
            position: 0,
        }
    }
}

impl Read for BlockingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            {
                let state = self.buffer.lock().unwrap();

                if let Some(ref err) = state.error {
                    return Err(io::Error::new(io::ErrorKind::Other, err.clone()));
                }

                if self.position < state.data.len() {
                    let available = &state.data[self.position..];
                    let to_read = buf.len().min(available.len());
                    buf[..to_read].copy_from_slice(&available[..to_read]);
                    self.position += to_read;
                    return Ok(to_read);
                }

                if state.complete {
                    return Ok(0); // EOF
                }
            }

            // Yield CPU to allow download task to progress
            // Using yield_now instead of sleep to avoid blocking the event loop
            std::thread::yield_now();
        }
    }
}

impl Seek for BlockingReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        loop {
            {
                let state = self.buffer.lock().unwrap();

                let target_pos = match pos {
                    SeekFrom::Start(offset) => offset as i64,
                    SeekFrom::Current(offset) => self.position as i64 + offset,
                    SeekFrom::End(offset) => {
                        if let Some(total) = state.total_size {
                            total as i64 + offset
                        } else if state.complete {
                            state.data.len() as i64 + offset
                        } else {
                            // Wait for completion to know size
                            drop(state);
                            std::thread::yield_now();
                            continue;
                        }
                    }
                };

                if target_pos < 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Seek to negative position",
                    ));
                }

                let target = target_pos as usize;

                // If seeking within buffered data, do it immediately
                if target <= state.data.len() || state.complete {
                    self.position = target;
                    return Ok(self.position as u64);
                }

                // Wait for more data
            }
            std::thread::yield_now();
        }
    }
}
