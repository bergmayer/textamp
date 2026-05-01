//! Library picker popup (F3).

use iced::widget::{button, column, container, Column};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::action::{Action, SearchAction, SettingsAction};
use crate::app::AppState;
use crate::ui_gui::message::GuiMessage;

use crate::ui_gui::widgets::text;
pub fn view<'a>(state: &'a AppState) -> Element<'a, GuiMessage> {
    let mut rows: Vec<Element<'a, GuiMessage>> = Vec::new();
    for (i, lib) in state.libraries.iter().enumerate() {
        let is_selected = i == state.popups.library_picker_index;
        // ASCII glyph — `\u{25B8}` (▸) doesn't render in every theme
        // / font combo and falls back to a box on Windows.
        let mark = if is_selected { "> " } else { "  " };
        let label = format!("{mark}{}", lib.title);
        let lib_key = lib.key.clone();

        // Each row is a button that both selects the library and closes
        // the popup — matching the keyboard path (`Enter` runs the same
        // two actions).
        rows.push(
            button(text(label).size(15))
                .width(Length::Fill)
                .padding([4, 8])
                .on_press_with(move || {
                    tracing::info!("library_picker: clicked {lib_key}");
                    GuiMessage::TabClick(vec![
                        Action::Settings(SettingsAction::SelectLibrary(lib_key.clone())),
                        Action::Search(SearchAction::CloseLibraryPicker),
                    ])
                })
                .style(move |theme: &Theme, status: button::Status| {
                    let p = theme.extended_palette();
                    let (bg, fg) = if is_selected {
                        (p.primary.strong.color, p.primary.strong.text)
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
    if rows.is_empty() {
        rows.push(text("No libraries discovered yet.").size(14).into());
    }
    container(
        column![
            text("Switch library").size(18),
            Column::with_children(rows).spacing(2),
        ]
        .spacing(10)
        .align_x(Alignment::Center),
    )
    .padding(18)
    .width(Length::Fixed(420.0))
    .style(|theme: &iced::Theme| {
        let p = theme.extended_palette();
        container::Style {
            background: Some(iced::Background::Color(p.background.base.color)),
            text_color: Some(p.background.base.text),
            border: iced::Border { color: p.primary.strong.color, width: 1.5, radius: 6.0.into() },
            ..container::Style::default()
        }
    })
    .into()
}
