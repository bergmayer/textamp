//! Related artists screen (grouped by relationship type).

use iced::widget::{column, container, scrollable, text, Column};
use iced::{Element, Length};

use crate::app::AppState;
use crate::ui_gui::message::GuiMessage;

pub fn view(state: &AppState) -> Element<'_, GuiMessage> {
    let groups = &state.related.groups;
    let mut rows: Vec<Element<'_, GuiMessage>> = Vec::with_capacity(groups.len() * 2);
    for g in groups {
        rows.push(text(format!("\u{2014} {}", g.artist.title)).size(13).into());
        for a in &g.albums {
            rows.push(text(format!("  {}", a.title)).size(12).into());
        }
    }
    if rows.is_empty() {
        rows.push(text("No related artists loaded.").size(13).into());
    }

    let body = scrollable(Column::with_children(rows).spacing(2))
        .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
        .height(Length::Fill);

    container(column![text("Related artists").size(14), body].spacing(8).padding(12))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
