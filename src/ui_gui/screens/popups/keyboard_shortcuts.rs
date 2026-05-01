//! Keyboard Shortcuts popup — same source text as the TUI Help view
//! and the GUI's full-screen Help, lifted from `util::help_text`.

use iced::widget::{button, column, container, row, scrollable, Space};
use iced::{Alignment, Background, Border, Element, Font, Length, Theme};

use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::text;
use crate::ui_gui::widgets::transport_bar::popout_button_style;
use crate::util::help_text::HELP_TEXT;

pub fn view<'a>() -> Element<'a, GuiMessage> {
    let close_btn = button(text("Close").size(14))
        .padding([4, 12])
        .on_press(GuiMessage::CloseKeyboardShortcuts)
        .style(popout_button_style);

    let header = row![
        text("Keyboard Shortcuts").size(20),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

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
        column![header, body].spacing(10),
    )
    .padding(18)
    .width(Length::Fixed(720.0))
    .height(Length::Fixed(620.0))
    .style(frame_style)
    .into()
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
