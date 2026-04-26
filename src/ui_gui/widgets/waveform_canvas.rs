//! Waveform visualization (Canvas).
//!
//! Reads `WaveformData.bins` — normalized 0..=1 amplitudes — and draws
//! mirrored vertical bars across the canvas. A playhead cursor tracks the
//! current playback position. No animation-driven redraw of the bar
//! geometry is needed; a `canvas::Cache` would further elide redraws but
//! the current bin count is small (~512) so we draw every frame cheaply.

use iced::mouse;
use iced::widget::canvas::{event, Event, Frame, Geometry, Path, Program, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Theme};

use crate::app::action::{Action, PlaybackAction};
use crate::services::WaveformData;
use crate::ui_gui::message::GuiMessage;

/// A canvas `Program` that paints a waveform.
pub struct Waveform<'a> {
    pub data: Option<&'a WaveformData>,
    pub position_ms: u64,
    pub duration_ms: u64,
}

impl<'a> Program<GuiMessage> for Waveform<'a> {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (event::Status, Option<GuiMessage>) {
        if self.duration_ms == 0 || bounds.width <= 0.0 {
            return (event::Status::Ignored, None);
        }
        let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event else {
            return (event::Status::Ignored, None);
        };
        let Some(pos) = cursor.position_in(bounds) else {
            return (event::Status::Ignored, None);
        };
        let ratio = (pos.x / bounds.width).clamp(0.0, 1.0);
        let target_ms = (ratio as f64 * self.duration_ms as f64) as u64;
        let msg = GuiMessage::Action(Action::Playback(PlaybackAction::Seek(target_ms)));
        (event::Status::Captured, Some(msg))
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        let palette = theme.extended_palette();
        let accent = palette.primary.base.color;
        let muted = palette.background.strong.color;

        let w = bounds.width;
        let h = bounds.height;
        let mid = h / 2.0;

        if let Some(data) = self.data {
            let n = w as usize;
            if n > 0 && !data.bins.is_empty() {
                let bins = data.resample(n.max(1));
                for (i, &amp) in bins.iter().enumerate() {
                    let x = i as f32 + 0.5;
                    let half_h = amp.clamp(0.0, 1.0) * (mid - 1.0);
                    let path = Path::line(
                        Point::new(x, mid - half_h),
                        Point::new(x, mid + half_h),
                    );
                    frame.stroke(&path, Stroke::default().with_color(muted).with_width(1.0));
                }
            }
        } else {
            // No data yet — a thin centre line hints the widget is live.
            let path = Path::line(Point::new(0.0, mid), Point::new(w, mid));
            frame.stroke(&path, Stroke::default().with_color(muted).with_width(1.0));
        }

        // Playhead cursor.
        if self.duration_ms > 0 {
            let ratio = (self.position_ms as f32 / self.duration_ms as f32).clamp(0.0, 1.0);
            let x = ratio * w;
            let path = Path::line(Point::new(x, 0.0), Point::new(x, h));
            frame.stroke(&path, Stroke::default().with_color(accent).with_width(2.0));
        }

        // Subtle fill over the played portion.
        let played_w = if self.duration_ms > 0 {
            w * (self.position_ms as f32 / self.duration_ms as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if played_w > 1.0 {
            let overlay = Color {
                r: accent.r, g: accent.g, b: accent.b, a: 0.08,
            };
            let rect = Path::rectangle(Point::ORIGIN, iced::Size::new(played_w, h));
            frame.fill(&rect, overlay);
        }

        vec![frame.into_geometry()]
    }
}
