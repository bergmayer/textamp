//! Artist bio popup (F4).

use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length};

use crate::app::state::ArtistBioPopup;
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::popout_button_style;

pub fn view<'a>(p: &'a ArtistBioPopup) -> Element<'a, GuiMessage> {
    let body: Element<'a, GuiMessage> = if p.loading {
        text("Loading bio\u{2026}").size(15).into()
    } else if p.bio.is_empty() {
        text("No bio available.").size(15).into()
    } else {
        scrollable(text(&p.bio).size(15)).height(Length::Fixed(360.0)).into()
    };
    let close_btn = button(text("Close").size(14))
        .padding([4, 12])
        .on_press(GuiMessage::CloseBioPopup)
        .style(popout_button_style);
    let header = row![
        text(&p.artist_name).size(20),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);
    container(
        column![header, body]
            .spacing(10),
    )
    .padding(18)
    .width(Length::Fixed(540.0))
    .style(|theme: &iced::Theme| {
        let p = theme.extended_palette();
        container::Style {
            background: Some(iced::Background::Color(p.background.base.color)),
            text_color: Some(p.background.base.text),
            border: iced::Border { color: p.primary.strong.color, width: 1.5, radius: 6.0.into() },
            ..container::Style::default()
        }
    })
    .into()
}
