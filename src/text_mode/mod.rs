//! In-process Ratatui output and libghostty-vt terminal rendering adapter.
pub mod backend;
pub mod vt;

/// Complete terminal presentation produced by the single application actor.
#[derive(Clone, Debug)]
pub struct PresentedFrame {
    pub identity: crate::app::windowed_presentation::Identity,
    pub serial: u64,
    pub viewport: backend::Viewport,
    pub frame: vt::Frame,
    pub breadcrumbs: Vec<String>,
    pub editor: Option<crate::app::terminal_text::Document>,
    pub mini_artwork: Vec<ratatui::layout::Rect>,
    pub sidebar_symbols: Vec<(ratatui::layout::Rect, &'static str)>,
    pub graphical_visualizer: Option<ratatui::layout::Rect>,
    pub transport_controls: Vec<(
        ratatui::layout::Rect,
        crate::app::transport_controls::TransportControl,
    )>,
    pub playing: bool,
    pub muted: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub input_region: Option<crate::app::presentation::TextInputRegion>,
}

/// Native events normalized to the existing terminal input vocabulary.
#[derive(Clone, Debug)]
pub enum Input {
    Edit(crate::app::terminal_text::Edit),
    Key(crossterm::event::KeyEvent),
    Mouse(crossterm::event::MouseEvent),
    Text(String),
    Paste(String),
    Focus(bool),
}
