//! Help screen with keybindings — shared text from `util::help_text`.

use crate::app::AppState;
use crate::ui::theme::theme;
use crate::util::help_text::{total_lines, HELP_TEXT};

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

/// Return the total number of lines in the help text (for scrollbar hit-testing).
pub fn help_total_lines() -> usize {
    total_lines()
}

pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area
    );

    let block = Block::default()
        .title(" help (↑↓ PgUp/PgDn to scroll, Esc to close) ")
        .title_style(Style::default().fg(t.colors.fg_accent))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.colors.border_focused))
        .style(Style::default().bg(t.colors.bg_primary));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Count lines for scroll clamping
    let line_count = HELP_TEXT.lines().count() as u16;
    let visible_lines = inner.height;
    let max_scroll = line_count.saturating_sub(visible_lines);
    let scroll = state.help_scroll.min(max_scroll);

    let paragraph = Paragraph::new(HELP_TEXT.trim())
        .style(Style::default().fg(t.colors.fg_primary))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, inner);

    // Scrollbar for long help text
    if line_count > visible_lines {
        crate::ui::widgets::render_scrollbar(
            frame, area,
            line_count as usize,
            visible_lines as usize,
            scroll as usize,
            None,
        );
    }
}
