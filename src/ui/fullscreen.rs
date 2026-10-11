//! Full-frame canvas: artwork in slot 1, then six visualizers; Escape returns.
use crate::ui::{theme::theme, RenderState};
use ratatui::{layout::Rect, style::Style, text::Span, widgets::Paragraph, Frame};

pub fn render(frame: &mut Frame, state: &RenderState, area: Rect) {
    if area.is_empty() {
        return;
    }
    let t = theme();
    let selected = crate::app::fullscreen::mode(state);
    let labels = [
        "1 Artwork",
        "2 Wave",
        "3 Spectrum",
        "4 Gram",
        "5 Scope",
        "6 Landscape",
        "7 Meters",
    ];
    let total: usize = labels.iter().map(|s| s.len() + 2).sum();
    let start = if total > area.width as usize {
        selected
    } else {
        0
    };
    let mut x = area.x;
    for offset in 0..7 {
        let index = (start + offset) % 7;
        let label = format!(" {} ", labels[index]);
        let width = (label.len() as u16).min(area.right().saturating_sub(x));
        if width == 0 {
            break;
        }
        let rect = Rect::new(x, area.y, width, 1);
        let style = if index == selected {
            Style::default()
                .fg(t.colors.title_focused)
                .bg(t.colors.bg_selection)
        } else {
            Style::default().fg(t.colors.fg_muted)
        };
        frame.render_widget(Paragraph::new(Span::styled(label, style)), rect);
        state
            .hit_regions
            .borrow_mut()
            .fullscreen_modes
            .push((rect, index));
        x += width;
        if x == area.right() {
            break;
        }
    }
    if area.height < 2 {
        return;
    }
    let footer = Rect::new(area.x, area.bottom() - 1, area.width, 1);
    frame.render_widget(
        Paragraph::new("Esc/F7 back · ←/→ or Tab modes · 1–7 · : commands")
            .style(Style::default().fg(t.colors.fg_muted)),
        footer,
    );
    state.hit_regions.borrow_mut().fullscreen_exit =
        Some(Rect::new(footer.x, footer.y, footer.width.min(11), 1));
    let canvas = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(2),
    );
    if state.fullscreen_artwork {
        super::screens::now_playing::render_current_artwork(frame, state, canvas);
    } else {
        super::screens::now_playing::render_visualizer_canvas(frame, state, canvas);
    }
}
