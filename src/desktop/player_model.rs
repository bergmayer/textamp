//! Immutable playback presentation and cached visualizer projections.
use crate::app::state::View;
#[derive(Clone, Default)]
pub struct Player {
    pub active: bool,
    pub fullscreen: bool,
    pub viewer: crate::app::state::DesktopFullscreenView,
    pub repeat: crate::app::state::RepeatMode,
    pub mode: usize,
    pub track: Option<crate::library::models::Track>,
    pub artwork: Option<std::sync::Arc<Vec<u8>>>,
    pub waveform: Vec<f32>,
    pub spectra: Vec<Vec<u8>>,
    pub xy: Vec<(f32, f32)>,
    pub meters: crate::app::meters::StudioMeters,
    pub message: String,
    pub station: String,
}
#[derive(Default)]
pub struct Projection {
    meters: super::meters::Display,
    artwork: Option<std::sync::Arc<Vec<u8>>>,
}
pub fn project(state: &crate::app::AppState, projection: &mut Projection) -> Player {
    let viewer = state.desktop_player_fullscreen;
    let active = matches!(state.view, View::Queue | View::NowPlaying) || viewer.is_open();
    let mode = if viewer.is_artwork() && !state.gui_multiple_windows {
        0
    } else {
        state.visualizer_tab as usize + 1
    };
    let artwork = state.current_artwork_bytes();
    if projection.artwork.as_deref().map(Vec::as_slice) != artwork {
        projection.artwork = artwork.map(|bytes| std::sync::Arc::new(bytes.to_vec()));
    }
    let mut model = Player {
        active,
        fullscreen: viewer.is_open(),
        viewer,
        repeat: state.queue.repeat,
        mode,
        track: state.current_track().cloned(),
        artwork: projection.artwork.clone(),
        station: state
            .station_starting
            .as_ref()
            .map(|s| format!("Loading {}…", s.title))
            .or_else(|| state.radio.active_station.as_ref().map(|s| s.title.clone()))
            .unwrap_or_default(),
        ..Default::default()
    };
    if !active
        && !state.desktop_split_view
        && !state.desktop_player_visible
        && !state.gui_multiple_windows
    {
        return model;
    }
    model.meters = projection.meters.update(
        &state.studio_meters,
        state.playback.request_id,
        state.playback.status == crate::app::PlayStatus::Playing,
        std::time::Instant::now(),
    );
    match mode {
        1 => {
            model.waveform = state
                .waveform
                .data()
                .map(|d| d.resample(800))
                .unwrap_or_default();
            model.message = state.waveform.error().cloned().unwrap_or_default();
            if state.waveform.generating() {
                model.message = state
                    .waveform
                    .progress()
                    .map(|p| p.message())
                    .unwrap_or_else(|| "Preparing waveform…".into());
            }
        }
        2..=3 | 5 => {
            if let Some(data) = state.spectrogram.data().filter(|d| d.frame_count > 0) {
                match mode {
                    2 => {
                        model.spectra.push(data.resample_spectrum(
                            data.frame_at_position(state.playback.position_ms),
                            96,
                        ))
                    }
                    3 => {
                        for frame in 0..192 {
                            model
                                .spectra
                                .push(data.resample_spectrum(frame * data.frame_count / 192, 64));
                        }
                    }
                    5 => {
                        for row in 0..32 {
                            let age = (31 - row) as u64 * 12_000 / 31;
                            let bins = if age > state.playback.position_ms {
                                vec![0; 64]
                            } else {
                                data.resample_spectrum(
                                    data.frame_at_position(state.playback.position_ms - age),
                                    64,
                                )
                            };
                            model.spectra.push(bins);
                        }
                    }
                    _ => {}
                }
            } else {
                model.message = state.spectrogram.error().cloned().unwrap_or_else(|| {
                    if state.spectrogram.generating() {
                        state
                            .spectrogram
                            .progress()
                            .map(|p| p.message())
                            .unwrap_or_else(|| "Preparing spectral data…".into())
                    } else {
                        String::new()
                    }
                });
            }
        }
        4 => {
            model.xy = state
                .vectorscope_buffer
                .iter()
                .step_by(4)
                .copied()
                .filter(|(l, r)| l.is_finite() && r.is_finite())
                .collect();
            if model.xy.is_empty() {
                model.message = if state.playback.status == crate::app::PlayStatus::Stopped {
                    ""
                } else {
                    "Live PCM is unavailable on this output."
                }
                .into();
            }
        }
        6 if !model.meters.available => {
            model.message = if state.playback.status == crate::app::PlayStatus::Stopped {
                ""
            } else {
                "Live PCM meters are unavailable on this output."
            }
            .into();
        }
        _ => {}
    }
    model
}
#[cfg(test)]
mod projection_tests {
    use super::{Projection, project};

    #[test]
    fn detached_visualizer_retains_waveform_when_player_is_closed_or_showing_artwork() {
        let mut state = crate::app::AppState::new();
        state.gui_multiple_windows = true;
        state.waveform.succeed(crate::media::WaveformData {
            track_key: "track".into(),
            duration_ms: 1000,
            bins: vec![0.5; 8],
            version: 1,
            created_at: 0,
        });
        let mut cache = Projection::default();
        assert!(!project(&state, &mut cache).waveform.is_empty());
        state.desktop_player_fullscreen = crate::app::state::DesktopFullscreenView::Artwork;
        let model = project(&state, &mut cache);
        assert!(!model.waveform.is_empty());
        assert!(model.viewer.is_artwork());
    }

    #[test]
    fn idle_visualizers_have_no_placeholder_prompt() {
        let mut state = crate::app::AppState::new();
        state.desktop_player_visible = true;
        let mut cache = Projection::default();
        for tab in crate::app::state::VisualizerTab::ALL {
            state.visualizer_tab = tab;
            assert!(project(&state, &mut cache).message.is_empty(), "{tab:?}");
        }
    }
}
