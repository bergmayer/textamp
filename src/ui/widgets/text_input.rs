//! Focused terminal fields with one source for text, selection and native caret geometry.
use crate::{
    app::{
        presentation::TextInputRegion,
        terminal_text::{self, Target},
    },
    ui::{RenderState, theme::theme},
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Draw the focused field when it owns the current input document.
/// Returns false when another control or overlay owns text input.
pub(crate) fn render(
    frame: &mut Frame,
    state: &RenderState<'_>,
    area: Rect,
    target: Target,
    style: Style,
) -> bool {
    let Some(document) =
        terminal_text::document(state).filter(|document| document.target == target)
    else {
        return false;
    };
    if area.is_empty() {
        return false;
    }
    let Some((line, region)) = layout(&document, area, style) else {
        return false;
    };
    frame.render_widget(Clear, area);
    frame.render_widget(Block::new().style(style), area);
    frame.render_widget(Paragraph::new(line).style(style), area);
    frame.set_cursor_position((region.column(document.selection.end), area.y));
    state.hit_regions.borrow_mut().text_input = Some(region);
    true
}

/// Fit a document to a single terminal row, retaining UTF-16 grapheme boundaries.
pub(crate) fn layout<'a>(
    document: &'a terminal_text::Document,
    area: Rect,
    style: Style,
) -> Option<(Line<'a>, TextInputRegion)> {
    if area.is_empty() {
        return None;
    }
    let colors = theme().colors;
    let mut units = 0;
    let graphemes: Vec<_> = document
        .text
        .graphemes(true)
        .map(|text| {
            let start = units;
            units += text.encode_utf16().count();
            (text, start, units, UnicodeWidthStr::width(text))
        })
        .collect();
    let caret = document.selection.end.min(units);
    let caret_index = graphemes
        .iter()
        .position(|(_, _, end, _)| *end > caret)
        .unwrap_or(graphemes.len());
    let mut start = caret_index;
    let mut width = 0;
    while start > 0 && width + graphemes[start - 1].3 < usize::from(area.width) {
        start -= 1;
        width += graphemes[start].3;
    }
    let offset = graphemes
        .get(start)
        .map_or(units, |(_, start, _, _)| *start);
    let mut carets = vec![(offset, area.x)];
    let mut spans = Vec::new();
    let mut x = 0;
    for &(text, begin, end, width) in &graphemes[start..] {
        if x + width > usize::from(area.width) {
            break;
        }
        let selected = begin < document.selection.end && end > document.selection.start;
        spans.push(Span::styled(
            text,
            if selected {
                style.bg(colors.bg_selection).fg(colors.selection_text)
            } else {
                style
            },
        ));
        x += width;
        carets.push((end, area.x + (x as u16).min(area.width - 1)));
    }
    let region = TextInputRegion {
        target: document.target.clone(),
        area: Rect::new(area.x, area.y, area.width, 1),
        carets,
    };
    Some((Line::from(spans), region))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::terminal_text::Document;

    #[test]
    fn input_geometry_tracks_graphemes_utf16_and_the_visible_caret() {
        let mut document = Document {
            target: Target::Palette,
            text: "ab界e\u{301}🦀z".into(),
            selection: 8..8,
            secret: false,
        };
        let area = Rect::new(10, 2, 6, 1);
        let (line, region) = layout(&document, area, Style::default()).unwrap();
        assert_eq!(line.to_string(), "e\u{301}🦀z");
        assert_eq!(region.column(8), 14);
        assert_eq!(region.offset(11), 5);
        assert!(
            region
                .carets
                .iter()
                .all(|(_, column)| *column < area.right())
        );

        document.selection = 2..3;
        let (_, region) = layout(&document, area, Style::default()).unwrap();
        assert_eq!(region.column(2), 12);
        assert_eq!(region.column(3), 14);
        assert_eq!(region.offset(13), 2);
        assert!(layout(&document, Rect::default(), Style::default()).is_none());
    }
}
