//! Optional landscape and studio displays; pure render-from-state.
use crate::app::meters::dbfs;
use crate::app::state::PlayStatus;
use crate::app::AppState;
use crate::ui::theme::theme;
use ratatui::prelude::*;
use ratatui::widgets::{
    canvas::{Canvas, Line as CanvasLine},
    Paragraph, Sparkline,
};

fn message(frame: &mut Frame, area: Rect, text: &str) {
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme().colors.fg_muted)),
        area,
    );
}

/// Past spectra recede toward a horizon; near rows show the current position.
pub fn landscape(frame: &mut Frame, state: &AppState, area: Rect) {
    if area.width < 12 || area.height < 4 {
        message(frame, area, "Landscape: enlarge pane");
        return;
    }
    let Some(data) = state
        .spectrogram
        .data
        .as_ref()
        .filter(|data| data.frame_count > 0 && data.bins_per_frame > 0)
    else {
        message(
            frame,
            area,
            if state.spectrogram.generating {
                "Preparing spectral landscape…"
            } else {
                "No spectral data — play a track to generate"
            },
        );
        return;
    };
    let bands = 48usize;
    let rows = (area.height as usize / 2).clamp(6, 16);
    let point = |row: usize, band: usize| {
        let depth = row as f64 / (rows - 1) as f64;
        let age = ((1.0 - depth) * 12_000.0) as u64;
        let position = state.playback.position_ms.saturating_sub(age);
        let magnitude = if age > state.playback.position_ms {
            0.0
        } else {
            f64::from(data.resampled_spectrum_peak(data.frame_at_position(position), bands, band))
                / 255.0
        };
        let spread = 0.14 + depth * 0.36;
        let x = 0.5 + (band as f64 / (bands - 1) as f64 * 2.0 - 1.0) * spread;
        let y = 0.82 - depth * 0.72 + magnitude.powf(1.6) * (0.06 + depth * 0.23);
        (x, y)
    };
    let canvas = Canvas::default()
        .marker(ratatui::symbols::Marker::Braille)
        .background_color(theme().colors.bg_primary)
        .x_bounds([0.0, 1.0])
        .y_bounds([0.0, 1.12])
        .paint(|ctx| {
            for row in 0..rows {
                let light = (row * 150 / (rows - 1)) as u8;
                let color = Color::Rgb(25 + light / 3, 65 + light, 90 + light);
                for band in 0..bands {
                    let (x1, y1) = point(row, band);
                    if band + 1 < bands {
                        let (x2, y2) = point(row, band + 1);
                        ctx.draw(&CanvasLine {
                            x1,
                            y1,
                            x2,
                            y2,
                            color,
                        });
                    }
                    if row + 1 < rows && band % 8 == 0 {
                        let (x2, y2) = point(row + 1, band);
                        ctx.draw(&CanvasLine {
                            x1,
                            y1,
                            x2,
                            y2,
                            color,
                        });
                    }
                }
            }
        });
    frame.render_widget(canvas, area);
}

/// Sampled pre-volume PCM levels, not LUFS, true peaks, or hardware output gain.
pub fn meters(frame: &mut Frame, state: &AppState, area: Rect) {
    if area.width < 28 || area.height < 7 {
        message(frame, area, "Enlarge pane");
        return;
    }

    let meters = &state.studio_meters;
    if !meters.available || state.playback.status == PlayStatus::Stopped {
        message(frame, area, "No live PCM");
        return;
    }
    let row = |offset, height| Rect::new(area.x, area.y + offset, area.width, height);
    let width = area.width.saturating_sub(4) as usize;
    let bar_height = if area.height >= 16 { 3 } else { 1 };
    for channel in 0..2 {
        let y = channel as u16 * (bar_height + 1);
        let rms = dbfs(meters.rms[channel]);
        let peak = dbfs(meters.peak[channel]);
        let hold = dbfs(meters.held[channel]);
        let label = if channel == 0 { "L" } else { "R" };
        frame.render_widget(
            Paragraph::new(format!("{label}  {rms:5.1}"))
                .style(Style::default().fg(theme().colors.fg_primary)),
            row(y, 1),
        );
        let mut spans = vec![Span::raw("   ")];
        for column in 0..width {
            let db = -60.0 + column as f32 / (width - 1).max(1) as f32 * 60.0;
            let color = if db >= -1.0 {
                Color::Red
            } else if db >= -6.0 {
                Color::Yellow
            } else {
                Color::Green
            };
            let held =
                (((hold + 60.0) / 60.0).clamp(0.0, 1.0) * (width - 1) as f32).round() as usize;
            let glyph = if column == held && meters.held[channel] > 0.0 {
                "│"
            } else if db < rms {
                "█"
            } else if db < peak {
                "━"
            } else {
                "·"
            };
            spans.push(Span::styled(
                glyph,
                Style::default().fg(if glyph == "·" {
                    theme().colors.fg_muted
                } else {
                    color
                }),
            ));
        }
        frame.render_widget(
            Paragraph::new(vec![Line::from(spans); bar_height as usize]),
            row(y + 1, bar_height),
        );
    }
    let correlation_row = 2 * (bar_height + 1);
    // An unlabelled bipolar rail keeps phase information without a caption.
    let marker = meters
        .correlation
        .map(|value| ((value.clamp(-1.0, 1.0) + 1.0) * 0.5 * (width - 1) as f32).round() as usize);
    let rail: String = (0..width)
        .map(|x| {
            if Some(x) == marker {
                '●'
            } else if x == width / 2 {
                '┼'
            } else {
                '─'
            }
        })
        .collect();
    frame.render_widget(
        Paragraph::new(format!("   {rail}")).style(Style::default().fg(theme().colors.fg_accent)),
        row(correlation_row, 1),
    );
    if area.height >= correlation_row + 5 {
        let history: Vec<_> = meters
            .history
            .iter()
            .skip(meters.history.len().saturating_sub(area.width as usize))
            .copied()
            .collect();
        frame.render_widget(
            Sparkline::default()
                .data(&history)
                .max(60)
                .style(Style::default().fg(theme().colors.fg_accent)),
            row(
                correlation_row + 3,
                (area.height - correlation_row - 4).min(3),
            ),
        );
    }
}
