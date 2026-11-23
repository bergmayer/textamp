//! Display ballistics for sampled PCM. This never changes the measured audio.
use crate::app::meters::StudioMeters;
use std::time::Instant;

#[derive(Default)]
pub struct Display {
    identity: Option<u64>,
    last: Option<Instant>,
    power: [f32; 2],
    peak: [f32; 2],
}
impl Display {
    pub fn update(
        &mut self,
        input: &StudioMeters,
        id: u64,
        playing: bool,
        now: Instant,
    ) -> StudioMeters {
        if self.identity != Some(id) || !input.available {
            *self = Self {
                identity: Some(id),
                ..Self::default()
            };
        }
        let elapsed = self
            .last
            .map(|last| now.saturating_duration_since(last).as_secs_f32());
        self.last = Some(now);
        let mut output = input.clone();
        if !playing {
            output.correlation = None;
        }
        for channel in 0..2 {
            let rms = if playing { input.rms[channel] } else { 0. };
            let peak = if playing { input.peak[channel] } else { 0. };
            // Average power (not dB or amplitude), 300 ms integration. The
            // instantaneous sample peak attacks immediately and falls at 20 dB/s.
            let retain = elapsed.map_or(0., |dt| (-dt / 0.3).exp());
            self.power[channel] = self.power[channel] * retain + rms * rms * (1. - retain);
            self.peak[channel] = peak.max(self.peak[channel] * 10_f32.powf(-elapsed.unwrap_or(0.)));
            output.rms[channel] = self.power[channel].sqrt();
            output.peak[channel] = self.peak[channel];
        }
        output
    }
}
pub fn fraction(amplitude: f32) -> f32 {
    ((crate::app::meters::dbfs(amplitude) + 60.) / 60.).clamp(0., 1.)
}
pub fn label(amplitude: f32) -> String {
    if amplitude > 0.001 && amplitude.is_finite() {
        format!("{:.1}", crate::app::meters::dbfs(amplitude))
    } else {
        "−∞".into()
    }
}

pub(super) fn geometry(height: f32) -> (f32, f32, f32, f32) {
    let top = if height < 160. { 12. } else { 18. };
    let history_bottom = (height - 18.).max(top + 8.);
    let history_top = history_bottom - (height * 0.06).clamp(4., 16.);
    let bottom = (history_top - 8.).max(top + 4.);
    (
        top / height.max(1.),
        (bottom - top) / height.max(1.),
        history_top / height.max(1.),
        history_bottom / height.max(1.),
    )
}

fn scale_ticks(height: f32) -> Vec<(i32, f32)> {
    let mut ticks = Vec::new();
    for db in [0, -6, -12, -24, -36, -48, -60] {
        let (top, span, _, _) = geometry(height);
        let y = height * (top + span * (-db as f32 / 60.));
        if db == -60 && ticks.last().is_some_and(|(_, last)| y - last < 16.) {
            ticks.pop();
        }
        if db == -60 || ticks.last().is_none_or(|(_, last)| y - last >= 16.) {
            ticks.push((db, y));
        }
    }
    ticks
}

pub(super) fn paint_labels(
    bounds: gpui_kit::Bounds<gpui_kit::Pixels>,
    color: gpui_kit::Hsla,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) {
    use gpui_kit::{SharedString, TextAlign, point, px};
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let mut draw = |text: String, x: f32, y: f32| {
        let mut style = window.text_style();
        style.color = color;
        let text: SharedString = text.into();
        let shaped = window.text_system().shape_line(
            text.clone(),
            px(10.),
            &[style.to_run(text.len())],
            None,
        );
        let _ = shaped.paint(
            bounds.origin + point(px(x), px(y)),
            px(14.),
            TextAlign::Left,
            None,
            window,
            cx,
        );
    };
    for (db, y) in scale_ticks(height) {
        draw(db.to_string(), width * 0.12, y - 7.);
    }
    if height >= 160. {
        draw("dBFS".into(), width * 0.12, 0.);
    } else {
        draw(
            "dBFS".into(),
            width * 0.02,
            height * geometry(height).0 - 7.,
        );
    }
    for (text, x) in [("L", 0.35), ("R", 0.57)] {
        draw(text.into(), width * x, 0.);
    }
    if height >= 160. {
        draw(
            "RMS · 12 s · −60…0 dBFS".into(),
            width * 0.02,
            height * geometry(height).2 - 7.,
        );
    }
    draw(
        "Correlation".into(),
        width * 0.02,
        (height * 0.90).min((height - 14.).max(0.)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn short_meter_panes_prioritize_bars_over_history() {
        for height in [100., 150., 240.] {
            let (top, bars, history_top, history_bottom) = geometry(height);
            assert!(bars > 0.5, "meter bars lost usable height at {height}");
            assert!(top + bars < history_top);
            assert!(history_bottom - history_top <= 0.07);
            assert!(history_bottom <= 1.);
        }
    }
    #[test]
    fn compact_scale_labels_have_readable_spacing_and_keep_endpoints() {
        for height in [100., 150., 240., 600.] {
            let ticks = scale_ticks(height);
            assert_eq!(ticks.first().unwrap().0, 0);
            assert_eq!(ticks.last().unwrap().0, -60);
            assert!(ticks.windows(2).all(|pair| pair[1].1 - pair[0].1 >= 16.));
        }
    }
    #[test]
    fn power_integration_peak_release_and_playback_identity() {
        let now = Instant::now();
        let mut measured = StudioMeters::default();
        measured.update(&[(0.5, 0.25)], now, 1);
        let mut display = Display::default();
        let first = display.update(&measured, 1, true, now);
        assert_eq!(first.rms, [0.5, 0.25]);
        measured.update(&[(0., 0.)], now + Duration::from_millis(300), 1);
        let next = display.update(&measured, 1, true, now + Duration::from_millis(300));
        assert!((next.rms[0] - 0.5 * (-1_f32 / 2.).exp()).abs() < 0.0001);
        assert!((next.peak[0] - 0.5 * 10_f32.powf(-0.3)).abs() < 0.0001);
        let reset = display.update(&measured, 2, true, now + Duration::from_millis(400));
        assert_eq!(reset.peak, [0., 0.]);
        assert_eq!(fraction(1.), 1.);
        assert_eq!(fraction(0.), 0.);
        assert_eq!(label(0.), "−∞");
    }
}

#[cfg(test)]
mod calibration_tests {
    use super::*;
    #[test]
    fn full_scale_sine_has_minus_three_db_rms_and_zero_db_sample_peaks() {
        let signal: Vec<_> = (0..4800)
            .map(|i| {
                let value = (i as f32 * std::f32::consts::TAU / 48.).sin();
                (value, value)
            })
            .collect();
        let mut measured = StudioMeters::default();
        let now = Instant::now();
        measured.update(&signal, now, 1);
        let displayed = Display::default().update(&measured, 1, true, now);
        assert!((crate::app::meters::dbfs(displayed.rms[0]) + 3.0103).abs() < 0.01);
        assert!((crate::app::meters::dbfs(displayed.peak[0])).abs() < 0.01);
        assert_eq!(displayed.correlation, Some(1.));
    }
}
