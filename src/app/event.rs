//! Application events.
//!
//! The top-level `Event` enum carries everything that flows through the
//! application's main channel. Payloads are grouped into sub-enums
//! (`DataEvent`, `PlaybackEvent`, etc.) defined in `event_core`.

pub use crate::app::event_core::*;

use crossterm::event::{KeyEvent, MouseEvent};
use tokio::sync::mpsc;

/// Top-level application event.
///
/// Every async task and the terminal input reader deposit values of this
/// type into the shared `mpsc::Sender<Event>`.
#[derive(Debug, Clone)]
pub enum Event {
    // Terminal input ----------------------------------------------------
    /// Raw terminal key press.
    Key(KeyEvent),
    /// Raw terminal mouse event.
    Mouse(MouseEvent),
    /// Terminal resized to (cols, rows).
    Resize(u16, u16),
    /// The terminal input source failed; stop instead of spinning forever.
    InputError(String),
    WorkerFailed(String),
    RadioResult {
        generation: u64,
        navigation_generation: Option<u64>,
        event: Box<Event>,
    },

    /// Completion command emitted by a background effect. Reducers remain
    /// single-threaded; only I/O and CPU work run outside the event loop.
    Effect(crate::app::Action),

    /// Result tied to the currently selected server/library pair.
    ///
    /// server section keys are only unique within a server (different servers
    /// commonly both use `"1"`).  Wrapping background results with this
    /// generation lets the event loop discard a completion from an earlier
    /// library context before it reaches any reducer.
    LibraryResult {
        generation: u64,
        event: Box<Event>,
    },

    /// Authentication and route-discovery completion scoped to the current
    /// account. Unlike `LibraryResult`, selecting a library does not invalidate
    /// this work.
    ConnectionResult {
        generation: u64,
        event: Box<Event>,
    },

    // Core events -------------------------------------------------------
    /// Periodic tick for animations/updates.
    Tick,

    Data(DataEvent),
    Playback(PlaybackEvent),
    Artwork(ArtworkEvent),

    Cache(CacheEvent),
    Visualizer(VisualizerEvent),
    Radio(RadioEvent),
    Playlist(PlaylistEvent),
    Ui(UiEvent),
}

impl From<crate::audio::AudioEvent> for Event {
    fn from(event: crate::audio::AudioEvent) -> Self {
        use crate::audio::AudioEvent;
        match event {
            AudioEvent::BufferingReady { playback_id } => {
                PlaybackEvent::BufferingEnd { playback_id }
            }
            AudioEvent::Error {
                playback_id,
                message,
            } => PlaybackEvent::PlaybackError {
                playback_id: Some(playback_id),
                message,
            },
            AudioEvent::SeekFailed {
                playback_id,
                message,
            } => PlaybackEvent::SeekFailed {
                playback_id,
                message,
            },
        }
        .into()
    }
}

impl Event {
    /// Scope an asynchronous completion to a server/library generation.
    pub fn for_library(generation: u64, event: impl Into<Event>) -> Self {
        Self::LibraryResult {
            generation,
            event: Box::new(event.into()),
        }
    }

    pub fn for_connection(generation: u64, event: impl Into<Event>) -> Self {
        Self::ConnectionResult {
            generation,
            event: Box::new(event.into()),
        }
    }
}

/// Sender facade that automatically scopes every completion to the library
/// generation that launched its task.
#[derive(Clone)]
pub(crate) struct LibraryEventSender {
    sender: mpsc::Sender<Event>,
    generation: u64,
    radio_generation: Option<u64>,
    navigation_generation: Option<u64>,
}

impl LibraryEventSender {
    pub(crate) fn new(sender: mpsc::Sender<Event>, generation: u64) -> Self {
        Self {
            sender,
            generation,
            radio_generation: None,
            navigation_generation: None,
        }
    }

    pub(crate) fn with_radio(mut self, generation: u64) -> Self {
        self.radio_generation = Some(generation);
        self
    }

    pub(crate) async fn send(&self, event: Event) -> Result<(), mpsc::error::SendError<Event>> {
        let event = match self.radio_generation {
            Some(generation) => Event::RadioResult {
                generation,
                navigation_generation: self.navigation_generation,
                event: Box::new(event),
            },
            None => event,
        };
        self.sender
            .send(Event::for_library(self.generation, event))
            .await
    }
}

// ============================================================================
// From impls for ergonomic construction
// ============================================================================

impl From<DataEvent> for Event {
    fn from(e: DataEvent) -> Self {
        Event::Data(e)
    }
}
impl From<PlaybackEvent> for Event {
    fn from(e: PlaybackEvent) -> Self {
        Event::Playback(e)
    }
}
impl From<ArtworkEvent> for Event {
    fn from(e: ArtworkEvent) -> Self {
        Event::Artwork(e)
    }
}

impl From<CacheEvent> for Event {
    fn from(e: CacheEvent) -> Self {
        Event::Cache(e)
    }
}
impl From<VisualizerEvent> for Event {
    fn from(e: VisualizerEvent) -> Self {
        Event::Visualizer(e)
    }
}
impl From<RadioEvent> for Event {
    fn from(e: RadioEvent) -> Self {
        Event::Radio(e)
    }
}
impl From<PlaylistEvent> for Event {
    fn from(event: PlaylistEvent) -> Self {
        Self::Playlist(event)
    }
}
impl From<UiEvent> for Event {
    fn from(e: UiEvent) -> Self {
        Event::Ui(e)
    }
}
