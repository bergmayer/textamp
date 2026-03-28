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
/// Layout:
/// ┌─────────────────────────────────────────────────┐
/// │ [Library] ^L library │ ^P playlists │ ^G genres  │  ← tab bar (1 row)
/// ├──────────────────────────────────────────────────┤
/// │  ┌──────────────┬──────────────────────────────┐ │
/// │  │ Category     │ Track List                   │ │
/// │  │ List         │ (grouped by album)           │ │
/// │  └──────────────┴──────────────────────────────┘ │
/// ├──────────────────────────────────────────────────┤
/// │ ▶ 00:00 ━━●──── 04:32 ⏮  ⏭ │ Track by Artist  │  ← transport (2 rows)
/// ├──────────────────────────────────────────────────┤
/// │ F1 help | F2 settings | F3 library | F5 refresh │  ← command bar row 1
/// │                                                  │  ← spacer row
/// │ ^E enqueue | ^S sort | ^W save playlist | ...   │  ← command bar row 2
/// └─────────────────────────────────────────────────┘
pub struct AppLayout {
    /// Left panel for category list (artists, albums, etc.)
    pub left_panel: Rect,
    /// Right panel for track list
    pub right_panel: Rect,
    /// Transport bar (now playing info, volume, time)
    pub transport: Rect,
    /// Command bar (always-visible alt commands, 3 rows)
    pub commands: Rect,
}

impl AppLayout {
    pub fn new(area: Rect) -> Self {
        // Split vertically: main content | transport | command bar
        // (tab bar removed — tabs are now in the command bar top row)
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),     // Main content
                Constraint::Length(2),  // Transport bar
                Constraint::Length(3),  // Command bar (3 rows: top + spacer + bottom)
            ])
            .split(area);

        // Split main content horizontally: left panel | right panel
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
            commands: main_chunks[2],
        }
    }
}

/// Layout for full-screen views (now playing queue, help, search).
pub struct FullScreenLayout {
    /// Main content area
    pub content: Rect,
    /// Transport bar
    pub transport: Rect,
    /// Command bar (always-visible, 3 rows)
    pub commands: Rect,
}

impl FullScreenLayout {
    pub fn new(area: Rect) -> Self {
        // Tab bar removed — tabs are now in the command bar top row
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),     // Content
                Constraint::Length(2),  // Transport
                Constraint::Length(3),  // Command bar (3 rows)
            ])
            .split(area);

        Self {
            content: chunks[0],
            transport: chunks[1],
            commands: chunks[2],
        }
    }
}
