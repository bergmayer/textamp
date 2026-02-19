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
    /// Tab bar at the top (navigation tabs + library name)
    pub tab_bar: Rect,
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
        // Split vertically: tab bar | main content | transport | command bar
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),  // Tab bar
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
            .split(main_chunks[1]);

        Self {
            tab_bar: main_chunks[0],
            left_panel: content_chunks[0],
            right_panel: content_chunks[1],
            transport: main_chunks[2],
            commands: main_chunks[3],
        }
    }
}

/// Layout for full-screen views (now playing queue, help, search).
pub struct FullScreenLayout {
    /// Tab bar at the top
    pub tab_bar: Rect,
    /// Main content area
    pub content: Rect,
    /// Transport bar
    pub transport: Rect,
    /// Command bar (always-visible, 3 rows)
    pub commands: Rect,
}

impl FullScreenLayout {
    pub fn new(area: Rect) -> Self {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),  // Tab bar
                Constraint::Min(5),     // Content
                Constraint::Length(2),  // Transport
                Constraint::Length(3),  // Command bar (3 rows)
            ])
            .split(area);

        Self {
            tab_bar: chunks[0],
            content: chunks[1],
            transport: chunks[2],
            commands: chunks[3],
        }
    }
}
