//! About Textamp popup — shows the Cubic-Player-style ANSI-art logo from
//! `textamp_clean.ansi`, app version, author, and a farewell tagline that
//! matches the TUI exit banner.

use iced::widget::text::{Rich, Span};
use iced::widget::{button, column, container, mouse_area, row, stack, Column, Space};
use iced::{Alignment, Background, Border, Color, Element, Font, Length, Theme};

use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::popout_button_style;

use crate::ui_gui::widgets::text;
const LOGO_ANSI: &str = include_str!("../../../../textamp_clean.ansi");

pub fn view<'a>() -> Element<'a, GuiMessage> {
    let version = env!("CARGO_PKG_VERSION");

    let logo = container(render_ansi(LOGO_ANSI))
        .padding([8, 12]);

    let divider = container(Space::with_height(Length::Fixed(1.0)))
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(theme.extended_palette().background.strong.color)),
            ..container::Style::default()
        });

    let meta = column![
        text(format!("textamp  v{version}")).size(18),
        text("A keyboard-driven Plex Music client").size(14),
        Space::with_height(Length::Fixed(8.0)),
        text("Terminal + desktop \u{2022} GPL-3.0 \u{2022} John Bergmayer").size(13),
        text("http://bergmayer.net/textamp").size(13)
            .color(Color { r: 0.30, g: 0.80, b: 0.90, a: 1.0 }),
        Space::with_height(Length::Fixed(8.0)),
        text("\u{201C}Why be bleak when you can be Blake?\u{201D}").size(13),
        text("  \u{2014} Jhon Balance").size(12),
    ]
    .spacing(2)
    .padding(12);

    let close_btn = button(text("Close").size(14))
        .padding([4, 16])
        .on_press(GuiMessage::HideAbout)
        .style(popout_button_style);

    let panel = container(
        column![
            logo,
            divider,
            meta,
            row![Space::with_width(Length::Fill), close_btn, Space::with_width(Length::Fixed(12.0))]
                .align_y(Alignment::Center),
            Space::with_height(Length::Fixed(12.0)),
        ]
        .spacing(0)
        .align_x(Alignment::Start),
    )
    .width(Length::Fixed(760.0))
    .style(|theme: &Theme| {
        let p = theme.extended_palette();
        container::Style {
            background: Some(Background::Color(Color::BLACK)),
            border: Border {
                color: p.primary.strong.color,
                width: 2.0,
                radius: 4.0.into(),
            },
            text_color: Some(p.background.base.text),
            ..container::Style::default()
        }
    });

    let backdrop = mouse_area(
        container(Space::with_width(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Color(Color { r: 0.0, g: 0.0, b: 0.0, a: 0.6 })),
                ..container::Style::default()
            }),
    )
    .on_press(GuiMessage::HideAbout);

    let centered = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}

// ── ANSI parsing ────────────────────────────────────────────────────────────
//
// Handles the subset of SGR codes the exit-banner file actually uses:
//   \x1b[0m               — reset colours
//   \x1b[38;2;R;G;Bm      — 24-bit foreground
//   \x1b[48;2;R;G;Bm      — 24-bit background
//   \x1b[?25l / [?25h     — cursor show/hide (ignored)
// Anything else is silently skipped; unparsable bytes are dropped.

fn render_ansi(input: &str) -> Element<'static, GuiMessage> {
    let mut lines: Vec<Element<'static, GuiMessage>> = Vec::new();
    for line in input.split('\n') {
        let spans = parse_line_to_spans(line);
        if spans.is_empty() {
            lines.push(text(" ".to_string()).size(13).font(Font::MONOSPACE).into());
        } else {
            lines.push(
                Rich::<'static, GuiMessage>::with_spans(spans)
                    .font(Font::MONOSPACE)
                    .size(13)
                    .into(),
            );
        }
    }
    Column::with_children(lines).spacing(0).into()
}

fn parse_line_to_spans(line: &str) -> Vec<Span<'static, GuiMessage>> {
    let mut spans: Vec<Span<'static, GuiMessage>> = Vec::new();
    let mut buf = String::new();
    let mut fg: Option<Color> = None;
    let mut bg: Option<Color> = None;
    let bytes = line.as_bytes();
    let mut i = 0usize;

    let flush = |spans: &mut Vec<Span<'static, GuiMessage>>,
                 buf: &mut String,
                 fg: Option<Color>,
                 bg: Option<Color>| {
        if buf.is_empty() { return; }
        let mut span = Span::new(std::mem::take(buf));
        if let Some(c) = fg { span = span.color(c); }
        if let Some(c) = bg { span = span.background(c); }
        spans.push(span);
    };

    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // ANSI CSI sequence — find the terminator.
            flush(&mut spans, &mut buf, fg, bg);
            let start = i + 2;
            let mut end = start;
            while end < bytes.len() && !matches!(bytes[end], b'm' | b'h' | b'l' | b'H' | b'J' | b'K') {
                end += 1;
            }
            if end < bytes.len() {
                let terminator = bytes[end];
                if terminator == b'm' {
                    // SGR parameter sequence
                    let params = &line[start..end];
                    apply_sgr(params, &mut fg, &mut bg);
                }
                // Other terminators (h/l/H/J/K) are cursor/display ops → ignore.
                i = end + 1;
                continue;
            } else {
                // Malformed — drop the rest of the line.
                break;
            }
        }
        // Decode one UTF-8 char starting at i.
        let remaining = &line[i..];
        if let Some(ch) = remaining.chars().next() {
            buf.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    flush(&mut spans, &mut buf, fg, bg);
    spans
}

fn apply_sgr(params: &str, fg: &mut Option<Color>, bg: &mut Option<Color>) {
    // Params are `;`-separated numeric tokens. Handle 0 / 38;2;R;G;B / 48;2;R;G;B.
    let nums: Vec<u16> = params.split(';').filter_map(|s| s.parse().ok()).collect();
    let mut i = 0;
    while i < nums.len() {
        match nums[i] {
            0 => { *fg = None; *bg = None; i += 1; }
            38 if i + 4 < nums.len() && nums[i + 1] == 2 => {
                *fg = Some(rgb(nums[i + 2], nums[i + 3], nums[i + 4]));
                i += 5;
            }
            48 if i + 4 < nums.len() && nums[i + 1] == 2 => {
                *bg = Some(rgb(nums[i + 2], nums[i + 3], nums[i + 4]));
                i += 5;
            }
            _ => { i += 1; }
        }
    }
}

fn rgb(r: u16, g: u16, b: u16) -> Color {
    Color {
        r: (r.min(255) as f32) / 255.0,
        g: (g.min(255) as f32) / 255.0,
        b: (b.min(255) as f32) / 255.0,
        a: 1.0,
    }
}
