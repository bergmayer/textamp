//! Similar tracks / albums / artists screen.

use iced::widget::{column, container, scrollable, text, Column};
use iced::{Element, Length};

use crate::app::AppState;
use crate::app::state::SimilarMode;
use crate::ui_gui::message::GuiMessage;

pub fn view(state: &AppState) -> Element<'_, GuiMessage> {
    let header = text(format!("Similar \u{2014} {:?}", state.similar.mode)).size(14);

    let rows: Vec<Element<'_, GuiMessage>> = match state.similar.mode {
        SimilarMode::Albums => state.similar.albums.iter().map(|a| {
            let label = format!("{}  \u{2014} {}", a.title, a.parent_title.as_deref().unwrap_or(""));
            text(label).size(13).into()
        }).collect(),
        SimilarMode::Tracks => state.similar.tracks.iter().map(|t| {
            text(format!("{}  \u{2014} {}", t.title, t.track_artist())).size(13).into()
        }).collect(),
        SimilarMode::Artists => state.similar.artists.iter().map(|a| {
            text(a.title.clone()).size(13).into()
        }).collect(),
    };

    let body = scrollable(Column::with_children(rows).spacing(2))
        .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
        .style(crate::ui_gui::widgets::chunky_scrollable_style)
        .height(Length::Fill);

    container(column![header, body].spacing(8).padding(12))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
