//! Bounded, sampled PCM metering. No device ownership or rendering here.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub fn dbfs(amplitude: f32) -> f32 {
    if amplitude.is_finite() && amplitude > 0.0 {
        (20.0 * amplitude.log10()).clamp(-60.0, 6.0)
    } else {
        -60.0
    }
}

#[derive(Debug, Clone, Default)]
pub struct StudioMeters {
    pub rms: [f32; 2],
    pub peak: [f32; 2],
    pub held: [f32; 2],
    pub correlation: Option<f32>,
    pub history: VecDeque<u64>,
    pub available: bool,
    playback_id: u64,
    last_sample: Option<Instant>,
    last_history: Option<Instant>,
    hold_until: [Option<Instant>; 2],
}

impl StudioMeters {
    pub fn update(&mut self, samples: &[(f32, f32)], now: Instant, playback_id: u64) {
        if self.playback_id != playback_id {
            *self = Self {
                playback_id,
                ..Self::default()
            };
        }
        let mut squares = [0.0f64; 2];
        let mut cross = 0.0f64;
        let mut peak = [0.0f32; 2];
        let mut count = 0;
        for &(l, r) in samples
            .iter()
            .filter(|(l, r)| l.is_finite() && r.is_finite())
        {
            count += 1;
            for (channel, value) in [l, r].into_iter().enumerate() {
                squares[channel] += f64::from(value).powi(2);
                peak[channel] = peak[channel].max(value.abs());
            }
            cross += f64::from(l) * f64::from(r);
        }
        if count == 0 {
            if self.last_sample.is_some_and(|last| {
                now.saturating_duration_since(last) > Duration::from_millis(250)
            }) {
                *self = Self {
                    playback_id,
                    ..Self::default()
                };
            }
            return;
        }
        let elapsed = self.last_sample.map_or(0.0, |last| {
            now.saturating_duration_since(last).as_secs_f32()
        });
        self.last_sample = Some(now);
        self.available = true;
        self.peak = peak;
        for (channel, square) in squares.iter().enumerate() {
            self.rms[channel] = (square / count as f64).sqrt() as f32;
            if self.peak[channel] >= self.held[channel] {
                self.held[channel] = self.peak[channel];
                self.hold_until[channel] = Some(now + Duration::from_millis(1500));
            } else if self.hold_until[channel].is_none_or(|until| now >= until) {
                // 12 dB/s release, independent of redraw speed.
                self.held[channel] =
                    self.peak[channel].max(self.held[channel] * 10f32.powf(-12.0 * elapsed / 20.0));
            }
        }
        let energy = (squares[0] * squares[1]).sqrt();
        self.correlation = (energy > 1e-12).then(|| (cross / energy).clamp(-1.0, 1.0) as f32);
        if self
            .last_history
            .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_millis(100))
        {
            self.last_history = Some(now);
            if self.history.len() == 120 {
                self.history.pop_front();
            }
            let rms = ((squares[0] + squares[1]) / (2 * count) as f64).sqrt() as f32;
            self.history
                .push_back((dbfs(rms) + 60.0).clamp(0.0, 60.0) as u64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn levels_and_correlation_are_measured_not_invented() {
        let mut meter = StudioMeters::default();
        let now = Instant::now();
        meter.update(&[(0.5, 0.5), (-0.5, -0.5)], now, 1);
        assert!((dbfs(meter.rms[0]) + 6.0206).abs() < 0.001);
        assert_eq!(meter.correlation, Some(1.0));
        meter.update(&[(0.5, -0.5), (-0.5, 0.5)], now, 1);
        assert_eq!(meter.correlation, Some(-1.0));
        meter.update(&[(0.0, 0.0)], now, 1);
        assert_eq!(meter.correlation, None);
        assert_eq!(dbfs(meter.rms[0]), -60.0);
    }
    #[test]
    fn holds_release_history_is_bounded_and_stale_samples_clear() {
        let mut meter = StudioMeters::default();
        let now = Instant::now();
        meter.update(&[(1.0, 1.0)], now, 1);
        meter.update(&[(0.1, 0.1)], now + Duration::from_secs(1), 1);
        assert_eq!(meter.held[0], 1.0);
        for i in 11..200 {
            meter.update(&[(0.1, 0.1)], now + Duration::from_millis(i * 100), 1);
        }
        assert_eq!(meter.history.len(), 120);
        assert!((meter.held[0] - 0.1).abs() < 0.001);
        meter.update(&[], now + Duration::from_secs(21), 1);
        assert!(!meter.available);
        meter.update(&[(f32::NAN, 1.0)], now + Duration::from_secs(22), 2);
        assert!(!meter.available);
        assert!(meter.history.is_empty());
    }
}
