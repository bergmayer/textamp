//! DJ Modes picker popup.
//!
//! Replaces the inline DJ-mode list that used to live in the Now
//! Playing sidebar. The user opens it from the "DJ Modes" sidebar
//! button; clicking a mode toggles it on/off (same `ToggleDjMode`
//! action) but leaves the popup open so the user can switch modes.
//! Clicking the active mode again clears it.

use iced::widget::{button, column, container, row, scrollable, Column, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::action::RadioAction;
use crate::app::state::{AppState, DjMode};
use crate::app::Action;
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::popout_button_style;

use crate::ui_gui::widgets::text;
pub fn view(state: &AppState) -> Element<'_, GuiMessage> {
    let active = state.dj.active_mode;
    let modes = [
        DjMode::Stretch,
        DjMode::Gemini,
        DjMode::Freeze,
        DjMode::Twofer,
        DjMode::Contempo,
        DjMode::Groupie,
    ];

    let close_btn = button(text("Close").size(14))
        .padding([4, 12])
        .on_press(GuiMessage::CloseDjModesPopup)
        .style(popout_button_style);

    let header = row![
        text("DJ Modes").size(20),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

    let rows: Vec<Element<'_, GuiMessage>> = modes.iter().map(|&m| {
        let is_active = active == Some(m);
        let prefix = if is_active { "\u{25CF} " } else { "  " };
        let label = format!("{prefix}{}", m.name());
        let descr = m.description().to_string();
        let body = column![
            text(label).size(15),
            text(descr).size(13),
        ]
        .spacing(2);
        button(body)
            .width(Length::Fill)
            .padding([6, 12])
            .on_press(GuiMessage::Action(Action::Radio(RadioAction::ToggleDjMode(m))))
            .style(move |theme: &Theme, status: button::Status| {
                let p = theme.extended_palette();
                let (bg, fg) = if is_active {
                    (p.primary.strong.color, p.primary.strong.text)
                } else {
                    match status {
                        button::Status::Hovered => (p.background.weak.color, p.background.weak.text),
                        _ => (Color::TRANSPARENT, p.background.base.text),
                    }
                };
                button::Style {
                    background: Some(Background::Color(bg)),
                    text_color: fg,
                    border: Border { color: p.background.strong.color, width: 1.0, radius: 3.0.into() },
                    ..button::Style::default()
                }
            })
            .into()
    }).collect();

    let list = scrollable(Column::with_children(rows).spacing(4))
        .height(Length::Fill);

    container(
        column![
            header,
            container(list)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(2)
                .style(|theme: &Theme| {
                    let p = theme.extended_palette();
                    container::Style {
                        border: Border { color: p.background.strong.color, width: 1.0, radius: 2.0.into() },
                        ..container::Style::default()
                    }
                }),
        ]
        .spacing(10),
    )
    .padding(18)
    .width(Length::Fixed(520.0))
    .height(Length::Fixed(440.0))
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
