//! Layout utilities for musikcube-style UI.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Create a centered rect with percentage of parent.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Main application layout.
///
/// The old two-row keybind footer is gone — every command it advertised
/// now lives in the `:` command palette (see `ui::command_palette`).
/// Reclaiming those rows means the library / queue panels each get
/// three more rows of usable height. Layout:
/// ```text
/// ┌─────────────────────────────────────────────────┐
/// │  Category list  │  Track list                   │  ← main content
/// ├──────────────────────────────────────────────────┤
/// │ ▶ 00:00 ━━●──── 04:32 ⏮  ⏭ │ Track by Artist  │  ← transport (2 rows)
/// └─────────────────────────────────────────────────┘
/// ```
pub struct AppLayout {
    /// Left panel for category list (artists, albums, etc.)
    pub left_panel: Rect,
    /// Right panel for track list
    pub right_panel: Rect,
    /// Transport bar (now playing info, volume, time)
    pub transport: Rect,
}

impl AppLayout {
    pub fn new(area: Rect) -> Self {
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),     // Main content
                Constraint::Length(2),  // Transport bar
            ])
            .split(area);

        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(30), // Left panel (category list)
                Constraint::Min(40),    // Right panel (track list)
            ])
            .split(main_chunks[0]);

        Self {
            left_panel: content_chunks[0],
            right_panel: content_chunks[1],
            transport: main_chunks[1],
        }
    }
}

/// Layout for full-screen views (now playing queue, help, search).
pub struct FullScreenLayout {
    /// Main content area
    pub content: Rect,
    /// Transport bar
    pub transport: Rect,
}

impl FullScreenLayout {
    pub fn new(area: Rect) -> Self {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),
                Constraint::Length(2),
            ])
            .split(area);

        Self {
            content: chunks[0],
            transport: chunks[1],
        }
    }
}
