//! Global search view — displays the results currently held in
//! `state.search.results`.
//!
//! Typing into the search field is handled by the shared
//! `handlers::key_input::handle_key` (the same path the TUI uses). Step 7
//! adds the modal-overlay chrome and tabbed result segmentation.

use iced::widget::{column, container, scrollable, Column};
use iced::{Element, Length};

use crate::app::AppState;
use crate::ui_gui::message::GuiMessage;

use crate::ui_gui::widgets::text;
pub fn view(state: &AppState) -> Element<'_, GuiMessage> {
    let query = text(format!("Query: {}", state.search.query)).size(14);

    let mut rows: Vec<Element<'_, GuiMessage>> = Vec::new();
    if let Some(results) = &state.search.results {
        if !results.artists.is_empty() {
            rows.push(text("\u{2014} Artists \u{2014}").size(14).into());
            for a in &results.artists {
                rows.push(text(format!("  {}", a.title)).size(14).into());
            }
        }
        if !results.albums.is_empty() {
            rows.push(text("\u{2014} Albums \u{2014}").size(14).into());
            for a in &results.albums {
                rows.push(text(format!("  {} \u{2014} {}", a.title, a.parent_title.as_deref().unwrap_or(""))).size(14).into());
            }
        }
        if !results.tracks.is_empty() {
            rows.push(text("\u{2014} Tracks \u{2014}").size(14).into());
            for t in &results.tracks {
                rows.push(text(format!("  {} \u{2014} {}", t.title, t.track_artist())).size(14).into());
            }
        }
    } else if !state.search.query.is_empty() {
        rows.push(text("(press Enter to execute search)").size(14).into());
    } else {
        rows.push(text("Type a query, then press Enter.").size(14).into());
    }

    let body = scrollable(Column::with_children(rows).spacing(2))
        .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
        .style(crate::ui_gui::widgets::chunky_scrollable_style)
        .height(Length::Fill);

    container(column![text("Search").size(18), query, body].spacing(8).padding(12))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
