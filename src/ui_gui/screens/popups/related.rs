//! "Related Artists" modal popup.
//!
//! Displays `state.related.groups` (populated by
//! `DataAction::LoadRelated`) as a list grouped by the related artist.
//! Clicking an artist closes the popup and navigates to that artist in
//! the library — matching the user's request that similar/related
//! clicks take them straight to the artist.

use iced::widget::{button, column, container, row as iced_row, scrollable, text, Column, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::AppState;
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::popout_button_style;

pub fn view(state: &AppState) -> Element<'_, GuiMessage> {
    let title_line = text(format!(
        "Related artists \u{2014} {}",
        if state.related.source_title.is_empty() { "(no source)" } else { state.related.source_title.as_str() }
    ))
    .size(18);

    let close_btn = button(text("Close").size(12))
        .padding([4, 12])
        .on_press(GuiMessage::CloseRelatedPopup)
        .style(popout_button_style);
    let header = iced_row![title_line, Space::with_width(Length::Fill), close_btn]
        .align_y(Alignment::Center);

    let list: Element<'_, GuiMessage> = if state.related.loading {
        container(text("Loading related\u{2026}").size(14))
            .padding(24)
            .center_x(Length::Fill)
            .into()
    } else if state.related.groups.is_empty() {
        container(text("No related artists found.").size(13))
            .padding(24)
            .center_x(Length::Fill)
            .into()
    } else {
        let mut rows: Vec<Element<'_, GuiMessage>> = Vec::new();
        for g in &state.related.groups {
            let artist_key = g.artist.rating_key.clone();
            let source_tag = match g.source {
                crate::app::state::RelatedSource::Plex => "",
                crate::app::state::RelatedSource::SimilarTag => " (similar tag)",
                crate::app::state::RelatedSource::Alias => " (alias)",
            };
            let label = format!("{}{}", g.artist.title, source_tag);
            rows.push(artist_row(label, artist_key));
            // Show up to 5 album titles underneath as context.
            for a in g.albums.iter().take(5) {
                rows.push(
                    container(text(format!("    \u{00B7} {}", a.title)).size(11))
                        .padding([2, 24])
                        .into(),
                );
            }
        }
        scrollable(Column::with_children(rows).spacing(2)).height(Length::Fill).into()
    };

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
    .width(Length::Fixed(720.0))
    .height(Length::Fixed(560.0))
    .style(frame_style)
    .into()
}

fn artist_row(label: String, artist_key: String) -> Element<'static, GuiMessage> {
    button(text(label).size(13))
        .width(Length::Fill)
        .padding([6, 12])
        .on_press(GuiMessage::NavigateToArtist { artist_key })
        .style(|theme: &Theme, status: button::Status| {
            let p = theme.extended_palette();
            let (bg, fg) = match status {
                button::Status::Hovered => (p.primary.weak.color, p.primary.strong.color),
                _ => (Color::TRANSPARENT, p.background.base.text),
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: fg,
                border: Border::default(),
                ..button::Style::default()
            }
        })
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
