//! Sort popup — pick a column sort mode.

use iced::widget::{button, column, container, row, text, Column, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::state::SortPopupState;
use crate::ui_gui::message::{GuiMessage, StatePopupKind};
use crate::ui_gui::widgets::transport_bar::popout_button_style;

pub fn view<'a>(p: &'a SortPopupState) -> Element<'a, GuiMessage> {
    use crate::app::state::SortPopupOption;
    let rows = p.options.iter().enumerate().map(|(i, o)| {
        let is_selected = i == p.selected_index;
        let mark = if is_selected { "> " } else { "  " };
        let label = match o {
            SortPopupOption::SortMode(m)    => format!("{:?}", m),
            SortPopupOption::Direction      => "Reverse direction".to_string(),
            SortPopupOption::Artwork        => "Toggle artwork".to_string(),
            SortPopupOption::GroupByAlbum   => "Group by album".to_string(),
        };
        button(text(format!("{mark}{label}")).size(13))
            .width(Length::Fill)
            .padding([4, 8])
            .on_press(GuiMessage::SortPopupClick(i))
            .style(move |theme: &Theme, status: button::Status| {
                let pal = theme.extended_palette();
                let (bg, fg) = if is_selected {
                    (pal.primary.strong.color, pal.primary.strong.text)
                } else {
                    match status {
                        button::Status::Hovered => (pal.background.weak.color, pal.background.weak.text),
                        _ => (Color::TRANSPARENT, pal.background.base.text),
                    }
                };
                button::Style {
                    background: Some(Background::Color(bg)),
                    text_color: fg,
                    border: Border::default(),
                    ..button::Style::default()
                }
            })
            .into()
    }).collect::<Vec<Element<'a, GuiMessage>>>();

    let close_btn = button(text("Close").size(12))
        .padding([4, 12])
        .on_press(GuiMessage::CloseStatePopup(StatePopupKind::Sort))
        .style(popout_button_style);
    let header = row![
        text(format!("Sort \u{2014} {}", p.column_title)).size(16),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

    container(
        column![
            header,
            Column::with_children(rows).spacing(2),
        ]
        .spacing(10),
    )
    .padding(18)
    .width(Length::Fixed(360.0))
    .style(frame_style)
    .into()
}

fn frame_style(theme: &iced::Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(iced::Background::Color(p.background.base.color)),
        text_color: Some(p.background.base.text),
        border: iced::Border { color: p.primary.strong.color, width: 1.5, radius: 6.0.into() },
        ..container::Style::default()
    }
}
