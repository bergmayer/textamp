//! Reusable GUI widgets — Miller column, track list, transport bar,
//! now-playing bar, visualizer canvases, marquee.

pub mod context_menu;
pub mod menu_bar;
pub mod miller_column;
pub mod spectrogram_canvas;
pub mod spectrum_canvas;
pub mod tab_strip;
pub mod transport_bar;
pub mod waveform_canvas;

/// Vertical scrollbar with a chunky drag handle. Iced's defaults
/// (10 px track / 10 px scroller) are too thin to grab comfortably
/// with a mouse — these values match a typical desktop scrollbar.
/// Use as `scrollable(content).direction(fat_vertical_scrollbar())`.
pub fn fat_vertical_scrollbar() -> iced::widget::scrollable::Direction {
    use iced::widget::scrollable::{Direction, Scrollbar};
    Direction::Vertical(
        Scrollbar::default()
            .width(16)
            .scroller_width(14)
            .margin(2),
    )
}
