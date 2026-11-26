//! Terminal workspace layouts with a separate fixed footer.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Windowed chrome measurements shared by the graphical and terminal adapters.
pub const TRANSPORT_HEIGHT: f32 = 100.;
pub const SIDEBAR_WIDTH: f32 = 280.;
/// Four artwork rows and up to three caption lines in a terminal album tile.
pub const ALBUM_TILE_ROWS: u16 = 7;
pub const SMALL_ARTWORK: f32 = 48.;

/// Earliest whole band that fits together with the band ending at `end`.
pub(in crate::ui) fn first_fitting_band(
    end: usize,
    viewport: usize,
    band_y: impl Fn(usize) -> usize,
) -> usize {
    let minimum = band_y(end).saturating_sub(viewport);
    let (mut first, mut last) = (0, end);
    while first < last {
        let mid = first + (last - first) / 2;
        if band_y(mid) < minimum {
            first = mid + 1;
        } else {
            last = mid;
        }
    }
    first.min(end.saturating_sub(1))
}

/// Responsive widths of the transport's track card and control groups.
pub fn transport_widths(available: f32) -> (f32, f32) {
    let wing = if available < 460. {
        80.
    } else if available < 620. {
        112.
    } else if available < 720. {
        144.
    } else {
        184.
    };
    (wing, (available - wing * 2. - 64.).clamp(0., 560.))
}

/// Live window dimensions retained when changing the presentation adapter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowGeometry {
    pub sidebar_width: u16,
    pub queue_width: u16,
    pub player_header: u16,
}

/// Default Now Playing header, including its transport, in logical pixels.
pub fn player_header(height: f32, width: f32) -> f32 {
    let visualizer = if height < 500. { 100. } else { 140. };
    let minimum = (TRANSPORT_HEIGHT + 32. + if height < 500. { 128. } else { 176. })
        .min((height - visualizer).max(TRANSPORT_HEIGHT));
    let maximum =
        (width.max(350.) * if width < 650. { 0.35 } else { 0.5 } + TRANSPORT_HEIGHT + 32.)
            .min(height - visualizer)
            .max(minimum)
            .min(900.);
    (height * 0.70).max(minimum).min(maximum)
}

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
/// Commands are available through the `:` palette. Layout:
/// ```text
/// ┌─────────────────────────────────────────────────┐
/// │  Category list  │  Track list                   │  ← main content
/// ├──────────────────────────────────────────────────┤
/// │ ▶ 00:00 ━━●──── 04:32 ⏮  ⏭ │ Track by Artist  │  ← transport
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
                Constraint::Min(5),    // Main content
                Constraint::Length(5), // Transport bar
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
            .constraints([Constraint::Min(5), Constraint::Length(5)])
            .split(area);

        Self {
            content: chunks[0],
            transport: chunks[1],
        }
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    #[test]
    fn narrow_workspaces_keep_both_control_groups_outside_the_track_card() {
        for available in [350., 460., 500., 620., 720., 1200.] {
            let (wing, card) = transport_widths(available);
            assert!(wing > 0. && card > 0.);
            assert!(wing * 2. + card + 64. <= available);
        }
    }
}
