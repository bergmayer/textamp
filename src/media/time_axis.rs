//! Time-axis labels for waveform visualizers.
pub fn time_ticks(duration_ms: u64, labels: usize) -> Vec<(f32, String)> {
    if duration_ms == 0 {
        return vec![(0., "0:00".into())];
    }
    let seconds = duration_ms as f64 / 1000.;
    let raw = seconds / (labels.max(2) - 1) as f64;
    let power = 10_f64.powf(raw.log10().floor());
    let step = [1., 2., 5., 10.]
        .into_iter()
        .map(|n| n * power)
        .find(|n| *n >= raw)
        .unwrap_or(raw)
        .max(0.001);
    let count = (seconds / step).floor() as usize;
    (0..=count.min(100))
        .map(|i| {
            let value = i as f64 * step;
            let whole = value.floor() as u64;
            let text = if step < 1. {
                format!("{}:{:04.1}", whole / 60, value % 60.)
            } else {
                format!("{}:{:02}", whole / 60, whole % 60)
            };
            ((value / seconds) as f32, text)
        })
        .collect()
}
#[cfg(test)]
mod tests {
    #[test]
    fn rulers_are_monotonic_and_bounded() {
        let ticks = super::time_ticks(195000, 6);
        assert_eq!(ticks[0].1, "0:00");
        assert!(ticks.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(ticks.iter().all(|(x, _)| *x <= 1.));
    }
}

/// Elapsed and remaining playback labels shared by both transport presentations.
pub fn transport_times(position_ms: u64, duration_ms: u64) -> (String, String) {
    let label = |ms: u64| format!("{}:{:02}", ms / 60_000, ms / 1000 % 60);
    (
        label(position_ms),
        format!("−{}", label(duration_ms.saturating_sub(position_ms))),
    )
}

#[cfg(test)]
mod transport_tests {
    #[test]
    fn elapsed_and_remaining_times_are_bounded_at_the_track_end() {
        assert_eq!(
            super::transport_times(12_000, 336_000),
            ("0:12".into(), "−5:24".into())
        );
        assert_eq!(
            super::transport_times(70_000, 60_000),
            ("1:10".into(), "−0:00".into())
        );
    }
}
