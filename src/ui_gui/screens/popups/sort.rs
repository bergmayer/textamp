//! Sort popup — pick a column sort mode.

use iced::widget::{button, column, container, row, Column, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::state::SortPopupState;
use crate::ui_gui::message::{GuiMessage, StatePopupKind};
use crate::ui_gui::widgets::transport_bar::popout_button_style;

pub fn view<'a>(p: &'a SortPopupState) -> Element<'a, GuiMessage> {
    use crate::app::state::{ColumnSortMode, SortPopupOption};
use crate::ui_gui::widgets::text;
    let rows = p.options.iter().enumerate().map(|(i, o)| {
        let is_selected = i == p.selected_index;
        let mark = if is_selected { "> " } else { "  " };
        let label = match o {
            SortPopupOption::SortMode(m)    => match m {
                ColumnSortMode::Default     => "Default".to_string(),
                ColumnSortMode::ByArtist    => "By artist".to_string(),
                ColumnSortMode::ByAlbum     => "By album".to_string(),
                ColumnSortMode::ByTitle     => "By title".to_string(),
                ColumnSortMode::ByDuration  => "By duration".to_string(),
                ColumnSortMode::Shuffled    => "Shuffled".to_string(),
            },
            SortPopupOption::Direction      => "Reverse direction".to_string(),
            SortPopupOption::Artwork        => "Show album artwork".to_string(),
            SortPopupOption::GroupByAlbum   => "Group by album".to_string(),
        };
        button(text(format!("{mark}{label}")).size(15))
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

    // Pin a fixed shrink-disabled width on the close button so a long
    // title can't squeeze it into a wrapped "Cl / os / e" stack —
    // also fix the title at "View Options" (the column name was just
    // visual noise that varied wildly in length).
    let close_btn = container(
        button(text("Close").size(14).wrapping(iced::widget::text::Wrapping::None))
            .padding([4, 12])
            .on_press(GuiMessage::CloseStatePopup(StatePopupKind::Sort))
            .style(popout_button_style),
    )
    .width(Length::Shrink);
    let header = row![
        text("View Options").size(18),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .spacing(8)
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
