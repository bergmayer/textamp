//! Settings as a modal popup.
//!
//! Re-uses `screens::settings::view` for the body and wraps it in a
//! popup frame with a Close button. Settings stay accessible without
//! the user navigating away from whatever view they were on.

use iced::widget::{button, column, container, row, text, Space};
use iced::{Alignment, Background, Border, Element, Length, Theme};

use crate::app::AppState;
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::popout_button_style;

pub fn view<'a>(state: &'a AppState, ui_scale: f32) -> Element<'a, GuiMessage> {
    let close_btn = button(text("Close").size(12))
        .padding([4, 12])
        .on_press(GuiMessage::CloseSettingsPopup)
        .style(popout_button_style);
    let header = row![
        text("Settings").size(18),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

    let body = super::super::settings::view(state, ui_scale);

    container(column![header, body].spacing(10))
        .padding(18)
        .width(Length::Fixed(640.0))
        .height(Length::Fixed(560.0))
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
