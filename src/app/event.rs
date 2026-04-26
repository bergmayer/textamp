//! Application events.
//!
//! The top-level `Event` enum carries everything that flows through the
//! application's main channel. Payloads are grouped into sub-enums
//! (`AuthEvent`, `DataEvent`, etc.) defined in `event_core` so the GUI can
//! reuse them without pulling in any terminal types.
//!
//! Terminal-only input variants (`Key`, `Mouse`, `Resize`) are gated on
//! `feature = "tui"`. Under `feature = "gui"` they don't exist, so GUI builds
//! never see a crossterm type.

// Re-export sub-enums so `use crate::app::event::*` keeps working unchanged
// for all existing handler code.
pub use crate::app::event_core::*;

#[cfg(feature = "tui")]
use crossterm::event::{KeyEvent, MouseEvent};

/// Top-level application event.
///
/// Every async task and the TUI input reader deposit values of this type
/// into the shared `mpsc::Sender<Event>`.
#[derive(Debug, Clone)]
pub enum Event {
    // Terminal input (TUI only) -----------------------------------------
    /// Raw terminal key press. TUI builds only.
    #[cfg(feature = "tui")]
    Key(KeyEvent),
    /// Raw terminal mouse event. TUI builds only.
    #[cfg(feature = "tui")]
    Mouse(MouseEvent),
    /// Terminal resized to (cols, rows). TUI builds only.
    #[cfg(feature = "tui")]
    Resize(u16, u16),

    // Core / portable events --------------------------------------------
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
