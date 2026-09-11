//! Help screen: keys, source-aware sidebar visibility, library/AudioMuse settings,
//! weekly source caches, shared analysis-load progress and capability-based radio/DJ
//! controls, track-seeded Sonic Radio and the random-artist catalog minimum
//! — browse collections stay in the sidebar, selection actions in the palette
//! — Ctrl+F / sidebar Search and arrow-key theme-logo previews (no `?` binding)
//! — shared `util::help_text`.

use crate::ui::theme::theme;
use crate::ui::RenderState as AppState;
use crate::util::help_text::for_library;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

pub fn render(frame: &mut Frame, state: &AppState, area: Rect) {
    let t = theme();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(t.colors.bg_primary)),
        area,
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
    let text = for_library(crate::app::sources::sonic::enabled(state));
    let paragraph = Paragraph::new(text.trim())
        .style(Style::default().fg(t.colors.fg_primary))
        .wrap(Wrap { trim: false });
    let region = crate::app::presentation::ScrollableTextRegion {
        area,
        lines: paragraph.line_count(inner.width),
        visible: inner.height as usize,
    };
    let scroll = state.help_scroll.min(region.max_scroll());
    let line_count = region.lines;
    let visible_lines = region.visible;
    state.hit_regions.borrow_mut().help = Some(region);
    let paragraph = paragraph.scroll((scroll, 0));

    frame.render_widget(paragraph, inner);

    // Scrollbar for long help text
    if line_count > visible_lines {
        crate::ui::widgets::render_scrollbar(
            frame,
            area,
            line_count,
            visible_lines,
            scroll as usize,
            None,
        );
    }
}
