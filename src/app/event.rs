//! Application events.
//!
//! The top-level `Event` enum carries everything that flows through the
//! application's main channel. Payloads are grouped into sub-enums
//! (`AuthEvent`, `DataEvent`, etc.) defined in `event_core`.

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

    /// Completion command emitted by a background effect. Reducers remain
    /// single-threaded; only I/O and CPU work run outside the event loop.
    Effect(crate::app::Action),

    /// Result tied to the currently selected Plex server/library pair.
    ///
    /// Plex section keys are only unique within a server (different servers
    /// commonly both use `"1"`).  Wrapping background results with this
    /// generation lets the event loop discard a completion from an earlier
    /// library context before it reaches any reducer.
    LibraryResult {
        generation: u64,
        event: Box<Event>,
    },

    // Core events -------------------------------------------------------
    /// Periodic tick for animations/updates.
    Tick,
    Auth(AuthEvent),
    Data(DataEvent),
    Playback(PlaybackEvent),
    Artwork(ArtworkEvent),
    Folder(FolderEvent),
    Preload(PreloadEvent),
    Cache(CacheEvent),
    Visualizer(VisualizerEvent),
    Radio(RadioEvent),
    Ui(UiEvent),
    Remote(RemoteEvent),
}

impl Event {
    /// Scope an asynchronous completion to a server/library generation.
    pub fn for_library(generation: u64, event: impl Into<Event>) -> Self {
        Self::LibraryResult {
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
}

impl LibraryEventSender {
    pub(crate) fn new(sender: mpsc::Sender<Event>, generation: u64) -> Self {
        Self { sender, generation }
    }

    pub(crate) async fn send(
        &self,
        event: Event,
    ) -> Result<(), mpsc::error::SendError<Event>> {
        self.sender
            .send(Event::for_library(self.generation, event))
            .await
    }
}

// ============================================================================
// From impls for ergonomic construction
// ============================================================================

impl From<AuthEvent>       for Event { fn from(e: AuthEvent)       -> Self { Event::Auth(e) } }
impl From<DataEvent>       for Event { fn from(e: DataEvent)       -> Self { Event::Data(e) } }
impl From<PlaybackEvent>   for Event { fn from(e: PlaybackEvent)   -> Self { Event::Playback(e) } }
impl From<ArtworkEvent>    for Event { fn from(e: ArtworkEvent)    -> Self { Event::Artwork(e) } }
impl From<FolderEvent>     for Event { fn from(e: FolderEvent)     -> Self { Event::Folder(e) } }
impl From<PreloadEvent>    for Event { fn from(e: PreloadEvent)    -> Self { Event::Preload(e) } }
impl From<CacheEvent>      for Event { fn from(e: CacheEvent)      -> Self { Event::Cache(e) } }
impl From<VisualizerEvent> for Event { fn from(e: VisualizerEvent) -> Self { Event::Visualizer(e) } }
impl From<RadioEvent>      for Event { fn from(e: RadioEvent)      -> Self { Event::Radio(e) } }
impl From<UiEvent>         for Event { fn from(e: UiEvent)         -> Self { Event::Ui(e) } }
impl From<RemoteEvent>     for Event { fn from(e: RemoteEvent)     -> Self { Event::Remote(e) } }
