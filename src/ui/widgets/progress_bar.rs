//! Custom progress bar widget.

use crate::ui::theme::theme;
use ratatui::prelude::*;

/// Render a waveform-style progress bar.
pub fn render_waveform(frame: &mut Frame, progress: f64, area: Rect) {
    let t = theme();
    let width = area.width as usize;
    let progress_pos = (width as f64 * progress) as usize;

    // Unicode block characters for waveform effect
    let waveform_chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

    let mut spans = Vec::new();

    for i in 0..width {
        // Generate pseudo-random height based on position
        let height_idx = (i * 7 + i * i * 3) % waveform_chars.len();
        let c = waveform_chars[height_idx];

        let style = if i < progress_pos {
            Style::default().fg(t.colors.fg_accent)
        } else {
            Style::default().fg(t.colors.border)
        };

        spans.push(Span::styled(c.to_string(), style));
    }

    let line = Line::from(spans);
    let para = ratatui::widgets::Paragraph::new(line);
    frame.render_widget(para, area);
}
