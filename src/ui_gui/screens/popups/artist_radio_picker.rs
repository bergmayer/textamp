//! Artist radio picker (multi-artist blend DJ picker).

use iced::widget::{button, column, container, row, scrollable, Column, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::state::{ArtistRadioPickerState, ArtistRadioPickerStep};
use crate::ui_gui::message::{GuiMessage, StatePopupKind};
use crate::ui_gui::widgets::transport_bar::popout_button_style;

use crate::ui_gui::widgets::text;
pub fn view<'a>(p: &'a ArtistRadioPickerState) -> Element<'a, GuiMessage> {
    let title = match p.step {
        ArtistRadioPickerStep::EnterCount  => "How many artists to blend?",
        ArtistRadioPickerStep::SelectArtists => "Select artists",
    };
    let close_btn = button(text("Close").size(14))
        .padding([4, 12])
        .on_press(GuiMessage::CloseStatePopup(StatePopupKind::ArtistRadioPicker))
        .style(popout_button_style);
    let header = row![
        text(title).size(18),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

    let body: Element<'a, GuiMessage> = match p.step {
        ArtistRadioPickerStep::EnterCount => {
            column![
                text(format!("Count: {}", p.count_input)).size(16),
                text("Type a number 1–12 then press Enter.").size(13),
            ]
            .spacing(8)
            .into()
        }
        ArtistRadioPickerStep::SelectArtists => {
            let mut rows: Vec<Element<'a, GuiMessage>> = Vec::new();

            // Selected artists — rendered at the top with a check mark,
            // clicking removes them (toggle semantics match the
            // keyboard Space/Enter path).
            if !p.selected_artists.is_empty() {
                rows.push(
                    container(text(format!(
                        "\u{2014} Selected ({}/{}) \u{2014}",
                        p.selected_artists.len(),
                        p.max_artists,
                    )).size(13))
                    .padding([4, 6])
                    .into()
                );
                for a in &p.selected_artists {
                    rows.push(row_button(
                        format!("\u{2713} {}", a.title),
                        find_toggle_index(p, &a.rating_key),
                        false,
                    ));
                }
            }

            rows.push(
                container(text("\u{2014} Available \u{2014}").size(13))
                    .padding([4, 6])
                    .into()
            );
            for (i, a) in p.filtered_artists.iter().take(60).enumerate() {
                rows.push(row_button(
                    format!("  {}", a.title),
                    i,
                    i == p.item_index,
                ));
            }
            scrollable(Column::with_children(rows).spacing(2)).height(Length::Fixed(360.0)).into()
        }
    };

    container(column![header, body].spacing(10))
        .padding(18)
        .width(Length::Fixed(480.0))
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

/// Find the filtered-artists index for a selected artist so clicking
/// the selected-list entry removes the right one via the toggle path.
fn find_toggle_index(p: &ArtistRadioPickerState, key: &str) -> usize {
    p.filtered_artists
        .iter()
        .position(|a| a.rating_key == key)
        .unwrap_or(0)
}

fn row_button(label: String, idx: usize, is_selected: bool) -> Element<'static, GuiMessage> {
    button(text(label).size(14))
        .width(Length::Fill)
        .padding([4, 10])
        .on_press(GuiMessage::ArtistRadioPickerClick(idx))
        .style(move |theme: &Theme, status: button::Status| {
            let p = theme.extended_palette();
            let (bg, fg) = if is_selected {
                (p.primary.strong.color, p.primary.strong.text)
            } else {
                match status {
                    button::Status::Hovered => (p.primary.weak.color, p.primary.strong.color),
                    _ => (Color::TRANSPARENT, p.background.base.text),
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
}
