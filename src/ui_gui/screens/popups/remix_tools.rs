//! Remix Tools popup.
//!
//! Hosts the queue-mutating actions that previously lived inline in
//! the Now Playing sidebar (Remix: Gemini / Twofer / Stretch /
//! Doppelganger / Shuffle) along with one-shot queue management
//! (Clear, Save as playlist). Clicking any entry runs the action
//! and closes the popup — these are one-shots, not toggles.

use iced::widget::{button, column, container, row, scrollable, text, Column, Space};
use iced::{Alignment, Background, Border, Element, Length, Theme};

use crate::app::action::QueueAction;
use crate::app::state::AppState;
use crate::app::Action;
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::popout_button_style;

pub fn view(state: &AppState) -> Element<'_, GuiMessage> {
    let close_btn = button(text("Close").size(12))
        .padding([4, 12])
        .on_press(GuiMessage::CloseRemixToolsPopup)
        .style(popout_button_style);

    let header = row![
        text("Remix Tools").size(18),
        Space::with_width(Length::Fill),
        close_btn,
    ]
    .align_y(Alignment::Center);

    // Clear Queue / Save Queue as Playlist were moved out of this
    // popup and into dedicated Now Playing sidebar buttons — they
    // aren't remix operations, so grouping them here was misleading.
    let shuffle_active = state.queue.shuffle_undo_queue.is_some();
    let entries: Vec<(&'static str, &'static str, QueueAction)> = vec![
        ("Remix: Gemini",       "Insert similar tracks between queue items",          QueueAction::RemixGemini),
        ("Remix: Twofer",       "Insert same-artist tracks between queue items",      QueueAction::RemixTwofer),
        ("Remix: Stretch",      "Insert sonic bridge tracks between queue items",     QueueAction::RemixStretch),
        ("Remix: Doppelganger", "Replace each track with a similar track by another artist", QueueAction::RemixDoppelganger),
        if shuffle_active {
            ("Undo Shuffle",    "Restore the queue to its pre-shuffle order",         QueueAction::RemixUndoShuffle)
        } else {
            ("Remix: Shuffle",  "Shuffle the current queue (undoable)",               QueueAction::RemixShuffle)
        },
    ];

    let rows: Vec<Element<'_, GuiMessage>> = entries
        .into_iter()
        .map(|(label, descr, action)| {
            let body = column![
                text(label.to_string()).size(13),
                text(descr.to_string()).size(11),
            ]
            .spacing(2);
            button(body)
                .width(Length::Fill)
                .padding([6, 12])
                .on_press(GuiMessage::RemixToolClick(Action::Queue(action)))
                .style(|theme: &Theme, status: button::Status| {
                    let p = theme.extended_palette();
                    let (bg, fg) = match status {
                        button::Status::Hovered => (p.background.weak.color, p.background.weak.text),
                        _ => (iced::Color::TRANSPARENT, p.background.base.text),
                    };
                    button::Style {
                        background: Some(Background::Color(bg)),
                        text_color: fg,
                        border: Border { color: p.background.strong.color, width: 1.0, radius: 3.0.into() },
                        ..button::Style::default()
                    }
                })
                .into()
        })
        .collect();

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
    .height(Length::Fixed(480.0))
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

