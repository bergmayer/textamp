//! Real-time spectrum analyzer (frequency bars for the current frame).

use iced::mouse;
use iced::widget::canvas::{Frame, Geometry, Path, Program};
use iced::{Color, Point, Rectangle, Renderer, Theme};

use crate::plex::SpectrogramData;

pub struct Spectrum<'a> {
    pub data: Option<&'a SpectrogramData>,
    pub position_ms: u64,
}

impl<'a, Msg> Program<Msg> for Spectrum<'a> {
    type State = ();

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

        let idx = d.frame_at_position(self.position_ms);
        let start = idx * d.bins_per_frame;
        let end = start + d.bins_per_frame;
        if end > d.frames.len() {
            return vec![frame.into_geometry()];
        }
        let spectrum = &d.frames[start..end];

        // Log-ish spacing by collapsing bins in groups of 4 to keep a visible bar count.
        let group = (d.bins_per_frame / 64).max(1);
        let bar_count = d.bins_per_frame / group;
        let bar_w = w / bar_count as f32;

        for i in 0..bar_count {
            // Average magnitude over the group.
            let mut sum: u32 = 0;
            for b in 0..group {
                sum += spectrum[i * group + b] as u32;
            }
            let avg = (sum / group as u32) as f32 / 255.0;
            let bar_h = avg * h;
            let x = i as f32 * bar_w;
            let rect = Path::rectangle(
                Point::new(x + 1.0, h - bar_h),
                iced::Size::new((bar_w - 2.0).max(1.0), bar_h),
            );
            let hue = i as f32 / bar_count as f32;
            // Gradient from primary to secondary across the spectrum.
            let c1 = palette.primary.base.color;
            let c2 = palette.secondary.base.color;
            let color = Color {
                r: c1.r * (1.0 - hue) + c2.r * hue,
                g: c1.g * (1.0 - hue) + c2.g * hue,
                b: c1.b * (1.0 - hue) + c2.b * hue,
                a: 0.9,
            };
            frame.fill(&rect, color);
        }

        vec![frame.into_geometry()]
    }
}
