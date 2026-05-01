//! User Guide popup — renders the project's `README-GUI.md` as a
//! scrollable modal with reflowing paragraphs.
//!
//! Hard line breaks in the source markdown (each line wrapped at
//! ~80 columns) are NOT preserved at render time: contiguous
//! non-blank lines that aren't headings or bullets collapse into
//! one wrapping `text` widget so the popup reads like a real
//! document instead of a fixed-column file dump. Headings and
//! bullets keep their structural breaks; blank lines become
//! paragraph spacing.

use iced::widget::{button, column, container, row, scrollable, Column, Space};
use iced::{Alignment, Background, Border, Element, Length, Theme};

use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::popout_button_style;

use crate::ui_gui::widgets::text;
const README_GUI: &str = include_str!("../../../../README-GUI.md");

const POPUP_WIDTH: f32 = 780.0;
const POPUP_HEIGHT: f32 = 680.0;
const TEXT_SIZE: u16 = 15;
const H1_SIZE: u16 = 26;
const H2_SIZE: u16 = 20;
const H3_SIZE: u16 = 17;
const BULLET_INDENT: f32 = 18.0;

pub fn view<'a>() -> Element<'a, GuiMessage> {
    let close_btn = button(text("Close").size(15))
        .padding([5, 16])
        .on_press(GuiMessage::CloseUserGuide)
        .style(popout_button_style);

    let header = row![
        text("User Guide").size(24),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

    let blocks = render_markdown(README_GUI);

    let body = scrollable(
        Column::with_children(blocks)
            .spacing(10)
            .padding([8, 28]),
    )
    .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
    .style(crate::ui_gui::widgets::chunky_scrollable_style)
    .height(Length::Fill);

    container(
        column![header, body].spacing(12),
    )
    .padding(20)
    .width(Length::Fixed(POPUP_WIDTH))
    .height(Length::Fixed(POPUP_HEIGHT))
    .style(frame_style)
    .into()
}

/// Walk the README a line at a time and emit document blocks. Each
/// block is one Element: a heading, a bullet, a paragraph, or a
/// horizontal divider for fenced-code boundaries. Continuation lines
/// inside a paragraph are merged with a space so iced's text widget
/// can wrap them naturally to the popup's width.
fn render_markdown<'a>(src: &str) -> Vec<Element<'a, GuiMessage>> {
    let mut out: Vec<Element<'a, GuiMessage>> = Vec::new();
    let mut paragraph: String = String::new();
    let mut in_code_fence = false;

    let flush_paragraph = |buf: &mut String, out: &mut Vec<Element<'a, GuiMessage>>| {
        if !buf.is_empty() {
            out.push(text(std::mem::take(buf)).size(TEXT_SIZE).into());
        }
    };

    for line in src.lines() {
        let trimmed = line.trim_end();

        if trimmed.starts_with("```") {
            // Fenced code blocks aren't reflowed — surface the
            // content verbatim in a thin separator-bracketed block.
            flush_paragraph(&mut paragraph, &mut out);
            in_code_fence = !in_code_fence;
            out.push(divider());
            continue;
        }
        if in_code_fence {
            // Drop fenced code wholesale; the README's code blocks
            // are duplicates of build-script invocations already
            // shown elsewhere in the GUI. Keeping them here would
            // distract from the prose.
            continue;
        }

        if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, &mut out);
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("# ") {
            flush_paragraph(&mut paragraph, &mut out);
            out.push(text(strip_md_emphasis(rest)).size(H1_SIZE).into());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            flush_paragraph(&mut paragraph, &mut out);
            out.push(text(strip_md_emphasis(rest)).size(H2_SIZE).into());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("### ") {
            flush_paragraph(&mut paragraph, &mut out);
            out.push(text(strip_md_emphasis(rest)).size(H3_SIZE).into());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- ") {
            flush_paragraph(&mut paragraph, &mut out);
            out.push(bullet(strip_md_emphasis(rest)));
            continue;
        }

        // Continuation of a paragraph — append with a space so the
        // newline in the source doesn't show up as a hard break.
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(&strip_md_emphasis(trimmed));
    }
    flush_paragraph(&mut paragraph, &mut out);
    out
}

/// One bullet row — fixed leading indent + the dot glyph + the
/// reflowing line of text. The text widget itself handles wrapping
/// to the popup width so long bullets break cleanly.
fn bullet<'a>(content: String) -> Element<'a, GuiMessage> {
    row![
        Space::with_width(Length::Fixed(BULLET_INDENT)),
        text("\u{2022}").size(TEXT_SIZE),
        Space::with_width(Length::Fixed(8.0)),
        text(content).size(TEXT_SIZE),
    ]
    .align_y(Alignment::Start)
    .into()
}

/// Faint horizontal rule used at fenced-code boundaries.
fn divider<'a>() -> Element<'a, GuiMessage> {
    container(Space::with_height(Length::Fixed(1.0)))
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(theme.extended_palette().background.strong.color)),
            ..container::Style::default()
        })
        .into()
}

/// Drop the markdown bold / italic / inline-code markers since iced's
/// plain `text` widget can't toggle them mid-string. Keeps the words.
fn strip_md_emphasis(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' => continue,
            '*' if chars.peek() == Some(&'*') => { chars.next(); }
            '*' => continue,
            '_' if chars.peek() == Some(&'_') => { chars.next(); }
            _ => out.push(c),
        }
    }
    out
}

fn frame_style(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(p.background.base.color)),
        text_color: Some(p.background.base.text),
        border: Border { color: p.primary.strong.color, width: 1.5, radius: 6.0.into() },
        ..container::Style::default()
    }
}
