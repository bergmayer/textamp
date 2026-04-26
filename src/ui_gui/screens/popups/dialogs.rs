//! Generic confirm / input dialog popups.
//!
//! Both dispatch through dedicated `GuiMessage` variants
//! (`ConfirmDialog*` / `InputDialog*`) so the handler can read the
//! active dialog's `on_confirm` or `action_type` from `AppState` at
//! click time and fire the correct follow-up action. This avoids the
//! previous bug where the Yes button hard-coded `SystemAction::Quit`
//! regardless of what the dialog was actually asking.

use iced::widget::{button, column, container, row, text, text_input};
use iced::{Alignment, Element, Length};

use crate::app::state::{ConfirmDialog, InputDialog};
use crate::ui_gui::message::GuiMessage;

pub fn confirm<'a>(d: &'a ConfirmDialog) -> Element<'a, GuiMessage> {
    let yes = button(text("Yes").size(13))
        .on_press(GuiMessage::ConfirmDialogYes)
        .padding([4, 14]);
    let no = button(text("No").size(13))
        .on_press(GuiMessage::ConfirmDialogNo)
        .padding([4, 14]);
    dialog_frame(
        column![
            text(&d.title).size(16),
            text(&d.message).size(13),
            row![yes, no].spacing(10).align_y(Alignment::Center),
        ]
        .spacing(12)
        .align_x(Alignment::Center),
    )
}

pub fn input<'a>(d: &'a InputDialog) -> Element<'a, GuiMessage> {
    let field = text_input("", &d.input)
        .on_input(GuiMessage::InputDialogChanged)
        .on_submit(GuiMessage::InputDialogSubmit)
        .padding(8)
        .width(Length::Fixed(320.0));
    let submit = button(text("OK").size(13))
        .on_press(GuiMessage::InputDialogSubmit)
        .padding([4, 14]);
    let cancel = button(text("Cancel").size(13))
        .on_press(GuiMessage::InputDialogCancel)
        .padding([4, 14]);
    dialog_frame(
        column![
            text(&d.title).size(16),
            field,
            row![submit, cancel].spacing(10).align_y(Alignment::Center),
        ]
        .spacing(12)
        .align_x(Alignment::Center),
    )
}

fn dialog_frame<'a>(inner: iced::widget::Column<'a, GuiMessage>) -> Element<'a, GuiMessage> {
    container(inner)
        .padding(20)
        .style(|theme: &iced::Theme| {
            let p = theme.extended_palette();
            container::Style {
                background: Some(iced::Background::Color(p.background.base.color)),
                text_color: Some(p.background.base.text),
                border: iced::Border {
                    color: p.primary.strong.color,
                    width: 1.5,
                    radius: 6.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}
