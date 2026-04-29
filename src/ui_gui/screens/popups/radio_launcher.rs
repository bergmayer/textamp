//! Radio launcher popup (search for seed, pick station).

use iced::widget::{button, column, container, row, scrollable, text, Column, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::state::RadioLauncherState;
use crate::ui_gui::message::{GuiMessage, StatePopupKind};
use crate::ui_gui::widgets::transport_bar::popout_button_style;

pub fn view<'a>(p: &'a RadioLauncherState) -> Element<'a, GuiMessage> {
    // Rows follow the layout `select_radio_launcher_result` expects:
    // artists first, then albums, then tracks — a flat `item_index`
    // runs through them.
    let mut rows: Vec<Element<'a, GuiMessage>> = Vec::new();
    let mut flat_idx: usize = 0;

    if let Some(r) = &p.results {
        if !r.artists.is_empty() {
            rows.push(section_header("Artists"));
            for a in &r.artists {
                rows.push(result_row(
                    format!("artist: {}", a.title),
                    flat_idx,
                    flat_idx == p.item_index,
                ));
                flat_idx += 1;
            }
        }
        if !r.albums.is_empty() {
            rows.push(section_header("Albums"));
            for a in &r.albums {
                rows.push(result_row(
                    format!("album:  {}", a.title),
                    flat_idx,
                    flat_idx == p.item_index,
                ));
                flat_idx += 1;
            }
        }
        if !r.tracks.is_empty() {
            rows.push(section_header("Tracks"));
            for t in &r.tracks {
                rows.push(result_row(
                    format!("track:  {}  \u{2014} {}", t.title, t.track_artist()),
                    flat_idx,
                    flat_idx == p.item_index,
                ));
                flat_idx += 1;
            }
        }
    } else if p.loading {
        rows.push(text("Searching\u{2026}").size(14).into());
    } else {
        rows.push(text("Type to search, Enter to seed radio.").size(14).into());
    }

    let close_btn = button(text("Close").size(14))
        .padding([4, 12])
        .on_press(GuiMessage::CloseStatePopup(StatePopupKind::RadioLauncher))
        .style(popout_button_style);
    let header = row![
        text(format!("Radio from: {}", p.query)).size(18),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

    container(
        column![
            header,
            scrollable(Column::with_children(rows).spacing(2)).height(Length::Fixed(360.0)),
        ]
        .spacing(10),
    )
    .padding(18)
    .width(Length::Fixed(520.0))
    .style(frame_style)
    .into()
}

fn section_header(label: &'static str) -> Element<'static, GuiMessage> {
    container(text(format!("\u{2014} {} \u{2014}", label)).size(13))
        .padding([4, 6])
        .into()
}

fn result_row(label: String, idx: usize, is_selected: bool) -> Element<'static, GuiMessage> {
    button(text(label).size(14))
        .width(Length::Fill)
        .padding([4, 10])
        .on_press(GuiMessage::RadioLauncherClick(idx))
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

fn frame_style(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(p.background.base.color)),
        text_color: Some(p.background.base.text),
        border: Border { color: p.primary.strong.color, width: 1.5, radius: 6.0.into() },
        ..container::Style::default()
    }
}
