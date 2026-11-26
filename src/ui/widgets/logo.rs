//! Shared ANSI wordmark and theme color mapping.
use crate::ui::theme::ThemeColors;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

/// Keep both foreground and background shading: the block characters alone
/// do not carry the artwork.
pub(crate) fn themed(colors: &ThemeColors) -> Vec<Line<'static>> {
    let mut lines = original(Color::Rgb(0, 0, 0));
    for span in lines.iter_mut().flat_map(|line| &mut line.spans) {
        span.style.fg = span.style.fg.map(|color| tint(color, colors));
        span.style.bg = span.style.bg.map(|color| tint(color, colors));
    }
    lines
}

fn tint(color: Color, colors: &ThemeColors) -> Color {
    fn rgb(color: Color) -> [u16; 3] {
        match color {
            Color::Rgb(r, g, b) => [r.into(), g.into(), b.into()],
            Color::White => [255; 3],
            _ => [0; 3],
        }
    }
    let [r, g, b] = rgb(color);
    let intensity = (54 * r + 183 * g + 19 * b) / 256;
    let background = rgb(colors.bg_primary);
    let accent = rgb(colors.fg_accent);
    let blended: [u8; 3] = std::array::from_fn(|i| {
        ((background[i] * (255 - intensity) + accent[i] * intensity) / 255) as u8
    });
    Color::Rgb(blended[0], blended[1], blended[2])
}

/// Choose a wordmark that fits without dropping cells from the artwork.
pub fn render_themed(frame: &mut Frame, area: Rect, colors: &ThemeColors) {
    frame.render_widget(Block::new().bg(colors.bg_primary), area);
    if area.is_empty() {
        return;
    }
    render_lines(frame, area, themed(colors), colors);
}

fn render_lines(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>, colors: &ThemeColors) {
    if area.is_empty() {
        return;
    }
    let width = lines.iter().map(Line::width).max().unwrap_or(0) as u16;
    let height = lines.len() as u16;
    if width == 0 || height == 0 {
        return;
    }
    if width <= area.width && height <= area.height {
        let target = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        Paragraph::new(lines)
            .style(Style::new().fg(colors.fg_accent).bg(colors.bg_primary))
            .render(target, frame.buffer_mut());
        return;
    }
    let compact = [
        "▀█▀ █▀▀ ▀▄▀ ▀█▀ ▄▀█ █▀▄▀█ █▀█",
        " █  ██▄ █ █  █  █▀█ █ ▀ █ █▀▀",
    ];
    let rows = if area.width >= compact[0].chars().count() as u16 && area.height >= 2 {
        compact.as_slice()
    } else if area.width >= 7 {
        &["textamp"]
    } else {
        return;
    };
    let target = Rect::new(
        area.x,
        area.y + (area.height - rows.len() as u16) / 2,
        area.width,
        rows.len() as u16,
    );
    frame.render_widget(
        Paragraph::new(rows.join("\n"))
            .centered()
            .style(Style::new().fg(colors.fg_accent).bg(colors.bg_primary)),
        target,
    );
}

pub fn render_search(frame: &mut Frame, area: Rect, colors: &ThemeColors) {
    let block = Block::new()
        .borders(Borders::ALL)
        .title(" search ")
        .border_style(Style::new().fg(colors.border))
        .style(Style::new().bg(colors.bg_primary).fg(colors.fg_accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).split(inner);
    render_themed(frame, rows[0], colors);
    frame.render_widget(
        Paragraph::new("press enter for search")
            .alignment(Alignment::Center)
            .style(Style::new().fg(colors.fg_primary)),
        rows[1],
    );
}

/// Parse the embedded ANSI art logo, replacing black with the theme background.
pub fn original(theme_bg: Color) -> Vec<Line<'static>> {
    let raw = include_str!("../../../assets/artwork/textamp_clean.ansi");
    let mut result = Vec::new();

    for line in raw.lines() {
        let spans = parse_ansi_line(line, theme_bg);
        if !spans.is_empty() {
            result.push(Line::from(spans));
        }
    }

    result
}

/// Parse a single line of ANSI escape sequences into ratatui styled spans.
fn parse_ansi_line(line: &str, theme_bg: Color) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current_text = String::new();
    let mut fg: Option<Color> = None;
    let mut bg: Option<Color> = None;

    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '[' {
            // Flush accumulated text as a span
            if !current_text.is_empty() {
                let mut style = Style::default();
                if let Some(c) = fg {
                    style = style.fg(c);
                }
                if let Some(c) = bg {
                    style = style.bg(c);
                }
                spans.push(Span::styled(std::mem::take(&mut current_text), style));
            }

            i += 2; // skip ESC[

            // Private mode sequences (e.g., ?25l cursor hide, ?25h cursor show)
            if i < chars.len() && chars[i] == '?' {
                while i < chars.len() && !chars[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
                continue;
            }

            // SGR sequence: collect params until 'm'
            let param_start = i;
            while i < chars.len() && chars[i] != 'm' {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }

            let param_str: String = chars[param_start..i].iter().collect();
            i += 1; // skip 'm'

            let params: Vec<u8> = param_str
                .split(';')
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();

            if params.is_empty() {
                fg = None;
                bg = None;
            } else {
                let mut j = 0;
                while j < params.len() {
                    match params[j] {
                        0 => {
                            fg = None;
                            bg = None;
                            j += 1;
                        }
                        38 if j + 4 < params.len() && params[j + 1] == 2 => {
                            let (r, g, b) = (params[j + 2], params[j + 3], params[j + 4]);
                            fg = Some(if r == 0 && g == 0 && b == 0 {
                                theme_bg
                            } else {
                                Color::Rgb(r, g, b)
                            });
                            j += 5;
                        }
                        48 if j + 4 < params.len() && params[j + 1] == 2 => {
                            let (r, g, b) = (params[j + 2], params[j + 3], params[j + 4]);
                            bg = Some(if r == 0 && g == 0 && b == 0 {
                                theme_bg
                            } else {
                                Color::Rgb(r, g, b)
                            });
                            j += 5;
                        }
                        _ => {
                            j += 1;
                        }
                    }
                }
            }
        } else {
            current_text.push(chars[i]);
            i += 1;
        }
    }

    // Flush remaining text
    if !current_text.is_empty() {
        let mut style = Style::default();
        if let Some(c) = fg {
            style = style.fg(c);
        }
        if let Some(c) = bg {
            style = style.bg(c);
        }
        spans.push(Span::styled(current_text, style));
    }

    spans
}
