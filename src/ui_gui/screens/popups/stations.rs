//! Radio / Stations picker popup.
//!
//! The queue view used to embed the full station tree in its left
//! sidebar with no visible way to back out of a drill. That surface has
//! been replaced with a "Radio…" button in the sidebar; the station
//! picker now lives here as a modal popup with a real Back/Close
//! affordance and a breadcrumb of the drill path.

use iced::widget::{button, column, container, row as iced_row, scrollable, Column, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::action::RadioAction;
use crate::app::state::AppState;
use crate::app::Action;
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::popout_button_style;

use crate::ui_gui::widgets::text;
pub fn view(state: &AppState) -> Element<'_, GuiMessage> {
    let nav = &state.station_nav;
    let focused_col = nav.focused();

    // Breadcrumb spans the drill path from root to the column currently
    // being viewed. `focused_column` is where the user is; columns
    // rightward exist in state but aren't visible when the user has
    // backed out (NavigateStationsBack only moves focus, doesn't drop
    // the deeper data).
    let visible_depth = nav.focused_column + 1;
    let breadcrumb_parts: Vec<String> = nav.columns.iter()
        .take(visible_depth)
        .map(|c| c.title.clone())
        .collect();
    let breadcrumb_str = if breadcrumb_parts.is_empty() {
        "Radio".to_string()
    } else {
        breadcrumb_parts.join("  \u{203A}  ")
    };

    let back_btn: Element<'_, GuiMessage> = if nav.focused_column > 0 {
        button(text("\u{2190} Back").size(14))
            .padding([4, 12])
            .on_press(GuiMessage::Action(Action::Radio(RadioAction::NavigateStationsBack)))
            .style(popout_button_style)
            .into()
    } else {
        Space::with_width(Length::Fixed(0.0)).into()
    };
    let close_btn = button(text("Close").size(14))
        .padding([4, 12])
        .on_press(GuiMessage::CloseStationsPopup)
        .style(popout_button_style);

    let toolbar = iced_row![
        back_btn,
        Space::with_width(Length::Fill),
        text(breadcrumb_str).size(14),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let mut rows: Vec<Element<'_, GuiMessage>> = Vec::new();
    if let Some(col) = focused_col {
        // This popup is just the radio picker. DJ modes, queue-edit
        // remixes, "action" pseudo-stations, and separator rows all
        // live in the Queue sidebar's "DJ Modes / Queue Tools" panel
        // now, so filter them out here.
        let radio_stations: Vec<(usize, &crate::plex::models::Station)> = col.stations.iter().enumerate()
            .filter(|(_, s)| {
                !s.is_dj_mode()
                    && !s.is_action()
                    && !s.is_separator()
                    && !s.is_remix()
            })
            .collect();
        if radio_stations.is_empty() {
            rows.push(text("(no stations)").size(14).into());
        } else {
            let active_key = state.radio.active_station.as_ref().map(|s| s.key.as_str());
            for (i, station) in radio_stations.iter().map(|(idx, s)| (*idx, *s)) {
                let is_selected = i == col.selected_index;
                let is_active = active_key == Some(station.key.as_str());
                let prefix = if is_active { "\u{266A} " } else { "  " };
                let suffix = if station.is_category() {
                    "  \u{203A}"
                } else {
                    ""
                };
                let label = format!("{prefix}{}{suffix}", station.title);
                let key = station.key.clone();
                let title = station.title.clone();

                // Categories drill, leaf stations call PlayStation (the
                // station-queue API). Using StartPlexRadio here is
                // wrong — that's the artist/album/track rating-key
                // radio API and Plex returns 0 tracks for a station
                // path, which surfaces as a misleading "Sonic Analysis
                // may be required" error.
                let msg = if station.is_category() {
                    GuiMessage::Action(Action::Radio(RadioAction::DrillIntoStation(key, title)))
                } else {
                    GuiMessage::PlayStationAndClose(vec![
                        Action::Radio(RadioAction::PlayStation(key)),
                    ])
                };

                rows.push(
                    button(text(label).size(15))
                        .width(Length::Fill)
                        .padding([6, 12])
                        .on_press(msg)
                        .style(move |theme: &Theme, status: button::Status| {
                            let p = theme.extended_palette();
                            let (bg, fg) = if is_selected {
                                (p.primary.strong.color, p.primary.strong.text)
                            } else if is_active {
                                (p.background.weak.color, p.primary.base.color)
                            } else {
                                match status {
                                    button::Status::Hovered => (p.background.weak.color, p.background.weak.text),
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
                        .into(),
                );
            }
        }
    } else {
        rows.push(text("Stations still loading\u{2026}").size(14).into());
    }

    let list = scrollable(Column::with_children(rows).spacing(0)).height(Length::Fill);

    container(
        column![
            text("Radio").size(20),
            toolbar,
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
    .width(Length::Fixed(560.0))
    .height(Length::Fixed(520.0))
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
