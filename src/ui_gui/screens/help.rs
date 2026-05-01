//! Help screen — keybindings from the shared `util::help_text`.
//! Both front-ends render the same source string so they can't drift.

use iced::widget::{column, container, scrollable};
use iced::{Background, Border, Color, Element, Font, Length, Theme};

use crate::app::AppState;
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::text;
use crate::util::help_text::HELP_TEXT;

pub fn view(_state: &AppState) -> Element<'_, GuiMessage> {
    let body = scrollable(
        container(
            text(HELP_TEXT.trim_start())
                .size(15)
                .font(Font::MONOSPACE),
        )
        .padding([4, 12]),
    )
    .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
    .style(crate::ui_gui::widgets::chunky_scrollable_style)
    .height(Length::Fill);

    container(
        column![
            text("Keyboard shortcuts  (Esc to close, ↑↓ PgUp/PgDn to scroll)").size(16),
            body,
        ]
        .spacing(8)
        .padding(12),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(|theme: &Theme| {
        let palette = theme.extended_palette();
        container::Style {
            background: Some(Background::Color(palette.background.base.color)),
            border: Border { color: palette.background.strong.color, width: 0.0, radius: 0.0.into() },
            text_color: Some(palette.background.base.text),
            ..container::Style::default()
        }
    })
    .into()
}

// Unused if iced silently strips it; kept to avoid wiring warnings.
#[allow(dead_code)]
const _IGNORE_COLOR: Color = Color::BLACK;
