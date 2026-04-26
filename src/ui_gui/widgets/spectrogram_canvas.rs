//! Full spectrogram heatmap across the track.

use iced::mouse;
use iced::widget::canvas::{event, Event, Frame, Geometry, Path, Program};
use iced::{Color, Point, Rectangle, Renderer, Theme};

use crate::app::action::{Action, PlaybackAction};
use crate::plex::SpectrogramData;
use crate::ui_gui::message::GuiMessage;

pub struct Spectrogram<'a> {
    pub data: Option<&'a SpectrogramData>,
    pub position_ms: u64,
}

impl<'a> Program<GuiMessage> for Spectrogram<'a> {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (event::Status, Option<GuiMessage>) {
        let Some(d) = self.data else { return (event::Status::Ignored, None); };
        if d.duration_ms == 0 || bounds.width <= 0.0 {
            return (event::Status::Ignored, None);
        }
        let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event else {
            return (event::Status::Ignored, None);
        };
        let Some(pos) = cursor.position_in(bounds) else {
            return (event::Status::Ignored, None);
        };
        let ratio = (pos.x / bounds.width).clamp(0.0, 1.0);
        let target_ms = (ratio as f64 * d.duration_ms as f64) as u64;
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

        let w = bounds.width;
        let h = bounds.height;

        let Some(d) = self.data else {
            return vec![frame.into_geometry()];
        };
        if d.bins_per_frame == 0 || d.frame_count == 0 {
            return vec![frame.into_geometry()];
        }

        let pixel_w = 2.0_f32;
        let column_count = ((w / pixel_w).max(1.0)) as usize;
        let frame_step = (d.frame_count / column_count).max(1);
        let pixel_h = h / d.bins_per_frame as f32;

        for col in 0..column_count {
            let frame_idx = col * frame_step;
            if frame_idx >= d.frame_count { break; }
            let start = frame_idx * d.bins_per_frame;
            let end = start + d.bins_per_frame;
            if end > d.frames.len() { break; }
            let spectrum = &d.frames[start..end];
            for (bin, &v) in spectrum.iter().enumerate() {
                if v == 0 { continue; }
                let intensity = v as f32 / 255.0;
                let color = inferno_color(intensity);
                // Invert bin index so low freq is at bottom.
                let y = h - (bin as f32 * pixel_h) - pixel_h;
                let rect = Path::rectangle(
                    Point::new(col as f32 * pixel_w, y),
                    iced::Size::new(pixel_w, pixel_h),
                );
                frame.fill(&rect, color);
            }
        }

        // Playhead cursor.
        if d.duration_ms > 0 {
            let ratio = (self.position_ms as f32 / d.duration_ms as f32).clamp(0.0, 1.0);
            let x = ratio * w;
            let accent = palette.primary.strong.color;
            let path = Path::line(Point::new(x, 0.0), Point::new(x, h));
            frame.stroke(
                &path,
                iced::widget::canvas::Stroke::default().with_color(accent).with_width(2.0),
            );
        }

        vec![frame.into_geometry()]
    }
}

/// Approximate `inferno` colormap (the matplotlib variant) — a
/// perceptually-uniform sequential ramp going black → purple → red →
/// orange → yellow that's the de-facto standard for scientific
/// spectrograms. Sampled at 9 stops and linearly interpolated.
fn inferno_color(t: f32) -> Color {
    const STOPS: &[(f32, f32, f32)] = &[
        (0.001462, 0.000466, 0.013866), // 0.000
        (0.094694, 0.044394, 0.241942), // 0.125
        (0.258234, 0.038571, 0.406485), // 0.250
        (0.428768, 0.072847, 0.432906), // 0.375
        (0.609330, 0.151848, 0.394891), // 0.500
        (0.783315, 0.247809, 0.300775), // 0.625
        (0.917603, 0.385323, 0.169580), // 0.750
        (0.984591, 0.578083, 0.068370), // 0.875
        (0.988362, 0.998364, 0.644924), // 1.000
    ];
    let t = t.clamp(0.0, 1.0);
    let scaled = t * (STOPS.len() - 1) as f32;
    let i = scaled.floor() as usize;
    let f = scaled - i as f32;
    let (r0, g0, b0) = STOPS[i.min(STOPS.len() - 1)];
    let (r1, g1, b1) = STOPS[(i + 1).min(STOPS.len() - 1)];
    Color {
        r: r0 + (r1 - r0) * f,
        g: g0 + (g1 - g0) * f,
        b: b0 + (b1 - b0) * f,
        a: 1.0,
    }
}
