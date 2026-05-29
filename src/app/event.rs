//! Application events.
//!
//! The top-level `Event` enum carries everything that flows through the
//! application's main channel. Payloads are grouped into sub-enums
//! (`AuthEvent`, `DataEvent`, etc.) defined in `event_core`.

pub use crate::app::event_core::*;

use crossterm::event::{KeyEvent, MouseEvent};

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
