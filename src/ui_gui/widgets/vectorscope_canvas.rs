//! Stereo vectorscope (Lissajous) visualizer.
//!
//! Plots a stream of stereo audio samples as an XY trace: the right
//! channel drives the X axis and the left channel drives the Y axis.
//! Samples are buffered in a fixed-size ring so the trace decays as
//! new audio arrives.
//!
//! Pure rendering — the audio-pipeline tap that calls
//! `Vectorscope::push_samples` lives in `src/audio/`; this widget
//! just paints whatever it's been given.

use iced::mouse;
use iced::widget::canvas::{
    self, Frame, Geometry, LineCap, LineJoin, Path, Program, Stroke,
};
use iced::{Color, Point, Rectangle, Renderer, Theme};

/// How many `(left, right)` sample tuples to retain. ~2 048 at 48 kHz
/// is roughly 43 ms of audio — long enough for the figure to look
/// like a continuous shape, short enough for the trace to "breathe"
/// with the music instead of caking up the centre.
const BUFFER_LEN: usize = 2_048;

/// Stereo Lissajous vectorscope state.
///
/// Owns the rolling sample buffer. Push fresh samples in via
/// `push_samples`; render via the `iced::widget::canvas::Program`
/// implementation.
#[derive(Debug, Clone)]
pub struct Vectorscope {
    /// Most-recent stereo samples. `(left_channel, right_channel)`,
    /// each in `[-1.0, 1.0]`. Oldest sample lives at `head`, newest
    /// at `head - 1` (wrapping). A length of `BUFFER_LEN` once the
    /// buffer has been filled.
    pub samples: Vec<(f32, f32)>,
    /// Write cursor — index of the next slot to overwrite. Wraps at
    /// `BUFFER_LEN` so push is O(1).
    head: usize,
}

impl Default for Vectorscope {
    fn default() -> Self {
        Self {
            samples: Vec::with_capacity(BUFFER_LEN),
            head: 0,
        }
    }
}

impl Vectorscope {
    /// Append a fresh batch of stereo samples to the rolling buffer.
    /// Overwrites the oldest entries once full so the buffer never
    /// grows past `BUFFER_LEN`.
    pub fn push_samples(&mut self, new_samples: &[(f32, f32)]) {
        if new_samples.is_empty() {
            return;
        }
        // Bring the buffer up to full length on first push without
        // a separate "warm-up" branch later.
        if self.samples.len() < BUFFER_LEN {
            let want = BUFFER_LEN - self.samples.len();
            let take = want.min(new_samples.len());
            self.samples.extend_from_slice(&new_samples[..take]);
            if take == new_samples.len() {
                return;
            }
            // Fell through with leftover samples; treat the rest as
            // an in-place overwrite from `head = 0`.
            let remaining = &new_samples[take..];
            for (i, s) in remaining.iter().enumerate() {
                self.samples[i % BUFFER_LEN] = *s;
            }
            self.head = remaining.len() % BUFFER_LEN;
            return;
        }
        // Steady-state: rotating overwrite.
        for s in new_samples {
            self.samples[self.head] = *s;
            self.head = (self.head + 1) % BUFFER_LEN;
        }
    }

    /// Iterate the buffer in chronological order (oldest first).
    /// Used by the renderer so the path traces out in the same order
    /// the audio arrived.
    fn ordered(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        let n = self.samples.len();
        let head = if n == BUFFER_LEN { self.head } else { 0 };
        (0..n).map(move |i| self.samples[(head + i) % n.max(1)])
    }
}

impl<Message> Program<Message> for Vectorscope {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());

        // Dark Mac-native field — the cyan trace pops against a
        // near-black background and the stacked-alpha glow reads
        // cleanly without competing with the canvas itself.
        let bg = Color::from_rgb8(30, 30, 30);
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), bg);

        let w = bounds.width;
        let h = bounds.height;
        if w <= 0.0 || h <= 0.0 || self.samples.len() < 2 {
            return vec![frame.into_geometry()];
        }

        // Map a sample to canvas coordinates.
        //   right channel ([-1, 1]) → X (0 .. w)
        //   left channel  ([-1, 1]) → Y (h .. 0)   (Y is inverted so
        //                                            +amp goes "up")
        let map = move |left: f32, right: f32| -> Point {
            let x = ((right.clamp(-1.0, 1.0) + 1.0) * 0.5) * w;
            let y = (1.0 - (left.clamp(-1.0, 1.0) + 1.0) * 0.5) * h;
            Point::new(x, y)
        };

        let mut iter = self.ordered();
        let first = match iter.next() {
            Some((l, r)) => map(l, r),
            None => return vec![frame.into_geometry()],
        };
        let path = Path::new(|builder| {
            builder.move_to(first);
            for (l, r) in iter {
                builder.line_to(map(l, r));
            }
        });

        // Slightly transparent stroke so heavily-overlapping segments
        // stack into a brighter glow at the centre — the "phosphor"
        // look characteristic of analog scopes.
        let trace = Color { r: 0.0, g: 200.0 / 255.0, b: 200.0 / 255.0, a: 0.8 };
        frame.stroke(
            &path,
            Stroke::default()
                .with_color(trace)
                .with_width(1.5)
                .with_line_cap(LineCap::Round)
                .with_line_join(LineJoin::Round),
        );

        vec![frame.into_geometry()]
    }
}
