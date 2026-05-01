//! Settings as a modal popup.
//!
//! Re-uses `screens::settings::view` for the body and wraps it in a
//! popup frame with a Close button. Settings stay accessible without
//! the user navigating away from whatever view they were on.

use iced::widget::{button, column, container, row, scrollable, Space};
use iced::{Alignment, Background, Border, Element, Length, Theme};

use crate::app::AppState;
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::popout_button_style;

use crate::ui_gui::widgets::text;
pub fn view<'a>(state: &'a AppState, ui_scale: f32) -> Element<'a, GuiMessage> {
    let close_btn = button(text("Close").size(14))
        .padding([4, 12])
        .on_press(GuiMessage::CloseSettingsPopup)
        .style(popout_button_style);
    let header = row![
        text("Settings").size(20),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

    // Wrap the body in a scrollable so newly-added sections (or
    // large theme lists) don't overflow the fixed popup height.
    // Without this the bottom rows render outside the popup
    // boundary and look clipped.
    let body = scrollable(super::super::settings::view(state, ui_scale))
        .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
        .style(crate::ui_gui::widgets::chunky_scrollable_style)
        .height(Length::Fill)
        .width(Length::Fill);

    // Fixed size large enough to fit the current section list
    // without scrolling on a typical desktop. The scrollable wrapper
    // is still there to handle pathological cases (very tall theme
    // list, future sections) but doesn't kick in for the default
    // content.
    container(column![header, body].spacing(10))
        .padding(18)
        .width(Length::Fixed(720.0))
        .height(Length::Fixed(760.0))
        .style(|theme: &Theme| {
            let p = theme.extended_palette();
            container::Style {
                background: Some(Background::Color(p.background.base.color)),
                text_color: Some(p.background.base.text),
                border: Border { color: p.primary.strong.color, width: 1.5, radius: 6.0.into() },
                ..container::Style::default()
            }
        })
        .into()
}
