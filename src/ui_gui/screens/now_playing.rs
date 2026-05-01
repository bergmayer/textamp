//! Now Playing view — full-screen album art, track metadata, visualizer.
//!
//! Switches between waveform / spectrum / spectrogram based on
//! `state.visualizer_tab`. Album art rendered from the grid cache if
//! available, otherwise a placeholder tile. Visualizers paint through
//! real Iced `Canvas` widgets (`widgets/waveform_canvas`, etc.) —
//! vector/GPU rendering, not ANSI block chars.

use iced::widget::canvas::Canvas;
use iced::widget::{button, column, container, image, row};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::state::VisualizerTab;
use crate::app::AppState;
use crate::ui_gui::images::lookup_grid;
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::fmt_ms;
use crate::ui_gui::widgets::vectorscope_canvas::Vectorscope;
use crate::ui_gui::widgets::{spectrogram_canvas, spectrum_canvas, text, waveform_canvas};

pub fn view<'a>(state: &'a AppState, vectorscope: &'a Vectorscope) -> Element<'a, GuiMessage> {
    let track = state.current_track();

    // Album artwork — prefer current_data (now-playing thumb), fall back
    // to grid_cache keyed by album rating key, else placeholder tile.
    let artwork: Element<'_, GuiMessage> = if let Some(bytes) = state.artwork.current_data.as_ref() {
        image(crate::ui_gui::images::handle_from_bytes(bytes))
            .width(Length::Fixed(260.0))
            .height(Length::Fixed(260.0))
            .into()
    } else if let Some(key) = track.and_then(|t| t.parent_rating_key.clone()) {
        if let Some(handle) = lookup_grid(&state.artwork.grid_cache, &key) {
            image(handle)
                .width(Length::Fixed(260.0))
                .height(Length::Fixed(260.0))
                .into()
        } else {
            placeholder_artwork()
        }
    } else {
        placeholder_artwork()
    };

    let title    = track.map(|t| t.title.clone()).unwrap_or_else(|| "No track playing".to_string());
    let artist   = track.map(|t| t.track_artist().to_string()).unwrap_or_default();
    let album    = track.and_then(|t| t.parent_title.clone()).unwrap_or_default();
    let year     = track.and_then(|t| t.year).map(|y| y.to_string()).unwrap_or_default();
    let position = fmt_ms(state.playback.position_ms);
    let duration = fmt_ms(state.playback.duration_ms);

    let metadata = column![
        text(title).size(26),
        text(artist).size(16),
        text(format!("{album}  {year}")).size(15),
        text(format!("{position} / {duration}")).size(14),
    ]
    .spacing(6);

    // Visualizer tab buttons — click to switch tab. Styled like the
    // primary tab strip (Library / Queue / Now Playing) but smaller.
    // Active tab shows an accent underline; inactive tabs show a subtle
    // hover highlight. Dispatches `GuiMessage::SetVisualizerTab` which
    // both flips `state.visualizer_tab` and triggers the matching data
    // load (waveform or spectrogram).
    let tab_btn = |label: &'static str, tab: VisualizerTab| -> Element<'_, GuiMessage> {
        let active = state.visualizer_tab == tab;
        let inner = container(text(label).size(14))
            .center_y(Length::Fixed(22.0))
            .center_x(Length::Fixed(100.0))
            .padding([0, 8]);
        button(inner)
            .width(Length::Fixed(100.0))
            .padding(0)
            .on_press(GuiMessage::SetVisualizerTab(tab))
            .style(move |theme: &Theme, status| {
                let p = theme.extended_palette();
                let (bg, fg) = if active {
                    (p.primary.strong.color, p.primary.strong.text)
                } else {
                    match status {
                        button::Status::Hovered => (p.background.weak.color, p.background.base.text),
                        _ => (Color::TRANSPARENT, p.background.base.text),
                    }
                };
                button::Style {
                    background: Some(Background::Color(bg)),
                    text_color: fg,
                    border: Border::default(),
                    ..button::Style::default()
                }
            })
            .into()
    };

    let viz_tabs = row![
        tab_btn("waveform",     VisualizerTab::Waveform),
        tab_btn("spectrum",     VisualizerTab::Spectrum),
        tab_btn("spectrogram",  VisualizerTab::Spectrogram),
        tab_btn("vectorscope", VisualizerTab::Vectorscope),
    ]
    .spacing(0);

    let viz_body = container(viz_canvas(state, vectorscope))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(8)
        .style(|theme: &Theme| {
            // Canvas bg = `background.base.color` so the bars (drawn
            // with `background.strong.color` in `waveform_canvas`) keep
            // their guaranteed-contrasting pair from the theme palette.
            // Hard-coding BLACK here used to look fine on Solarized
            // Dark but turned the Black-and-White theme's pure-black
            // bars invisible against a pure-black canvas.
            let p = theme.extended_palette();
            container::Style {
                background: Some(Background::Color(p.background.base.color)),
                border: Border {
                    color: p.background.strong.color,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..container::Style::default()
            }
        });

    let right_side = column![metadata, viz_tabs, viz_body]
        .spacing(12)
        .width(Length::Fill)
        .height(Length::Fill);

    let main = row![artwork, right_side]
        .spacing(24)
        .align_y(Alignment::Start)
        .padding(24);

    container(main)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Reusable visualizer panel — the visualizer-tab strip plus the
/// canvas itself, sharing the same `state.visualizer_tab` selection
/// the standalone Now Playing view uses. Exposed so the unified Now
/// Playing / Queue screen can host the visualizer in its bottom half
/// without duplicating the canvas wiring.
pub fn visualizer_panel<'a>(state: &'a AppState, vectorscope: &'a Vectorscope) -> Element<'a, GuiMessage> {
    let tab_btn = |label: &'static str, tab: VisualizerTab| -> Element<'_, GuiMessage> {
        let active = state.visualizer_tab == tab;
        let inner = container(text(label).size(14))
            .center_y(Length::Fixed(22.0))
            .center_x(Length::Fixed(100.0))
            .padding([0, 8]);
        button(inner)
            .width(Length::Fixed(100.0))
            .padding(0)
            .on_press(GuiMessage::SetVisualizerTab(tab))
            .style(move |theme: &Theme, status| {
                let p = theme.extended_palette();
                let (bg, fg) = if active {
                    (p.primary.strong.color, p.primary.strong.text)
                } else {
                    match status {
                        button::Status::Hovered => (p.background.weak.color, p.background.base.text),
                        _ => (Color::TRANSPARENT, p.background.base.text),
                    }
                };
                button::Style {
                    background: Some(Background::Color(bg)),
                    text_color: fg,
                    border: Border::default(),
                    ..button::Style::default()
                }
            })
            .into()
    };

    let tabs = row![
        tab_btn("waveform",     VisualizerTab::Waveform),
        tab_btn("spectrum",     VisualizerTab::Spectrum),
        tab_btn("spectrogram",  VisualizerTab::Spectrogram),
        tab_btn("vectorscope", VisualizerTab::Vectorscope),
    ]
    .spacing(0);

    let body = container(viz_canvas(state, vectorscope))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(8)
        .style(|theme: &Theme| {
            // Same fix as the Now Playing variant above — theme bg
            // pair instead of hard BLACK so monochrome themes don't
            // collapse the bars onto an identical-colour canvas.
            let p = theme.extended_palette();
            container::Style {
                background: Some(Background::Color(p.background.base.color)),
                border: Border {
                    color: p.background.strong.color,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..container::Style::default()
            }
        });

    column![tabs, body]
        .spacing(4)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Resolve the active visualizer tab to a live `Canvas` widget, or a
/// centered "Generating…/No data" message when the underlying data
/// hasn't been computed yet.
fn viz_canvas<'a>(state: &'a AppState, vectorscope: &'a Vectorscope) -> Element<'a, GuiMessage> {
    // With no current track the visualizer has nothing to be paused
    // against — even if `waveform.data` still holds the last track's
    // bins from cache, showing them would be misleading.
    if state.current_track().is_none() {
        return viz_placeholder("No track loaded");
    }
    match state.visualizer_tab {
        VisualizerTab::Waveform => {
            if state.waveform.data.is_none() {
                return viz_placeholder(if state.waveform.generating {
                    "Generating waveform\u{2026}"
                } else {
                    "No waveform data"
                });
            }
            let program = waveform_canvas::Waveform {
                data: state.waveform.data.as_ref(),
                position_ms: state.playback.position_ms,
                duration_ms: state.playback.duration_ms,
            };
            Canvas::new(program)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
        VisualizerTab::Spectrum => {
            if state.spectrogram.data.is_none() {
                return viz_placeholder(if state.spectrogram.generating {
                    "Generating spectrum\u{2026}"
                } else {
                    "No spectrum data"
                });
            }
            let program = spectrum_canvas::Spectrum {
                data: state.spectrogram.data.as_ref(),
                position_ms: state.playback.position_ms,
            };
            Canvas::new(program)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
        VisualizerTab::Spectrogram => {
            if state.spectrogram.data.is_none() {
                return viz_placeholder(if state.spectrogram.generating {
                    "Generating spectrogram\u{2026}"
                } else {
                    "No spectrogram data"
                });
            }
            let program = spectrogram_canvas::Spectrogram {
                data: state.spectrogram.data.as_ref(),
                position_ms: state.playback.position_ms,
            };
            Canvas::new(program)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
        VisualizerTab::Vectorscope => {
            // Live stereo Lissajous (right→X, left→Y). The rolling
            // sample buffer is fed by the audio sample tap inside
            // `App::handle_tick`; we clone it here because Canvas
            // takes its `Program` by value.
            Canvas::new(vectorscope.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
    }
}

fn viz_placeholder<'a>(msg: &'a str) -> Element<'a, GuiMessage> {
    container(text(msg).size(15))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(|theme: &iced::Theme| {
            let p = theme.extended_palette();
            container::Style {
                text_color: Some(p.background.weak.text),
                ..container::Style::default()
            }
        })
        .into()
}

fn placeholder_artwork<'a>() -> Element<'a, GuiMessage> {
    container(text("no cover").size(15))
        .width(Length::Fixed(260.0))
        .height(Length::Fixed(260.0))
        .center_x(Length::Fixed(260.0))
        .center_y(Length::Fixed(260.0))
        .style(|theme: &iced::Theme| {
            let p = theme.extended_palette();
            container::Style {
                background: Some(iced::Background::Color(p.background.weak.color)),
                text_color: Some(p.background.weak.text),
                ..container::Style::default()
            }
        })
        .into()
}
