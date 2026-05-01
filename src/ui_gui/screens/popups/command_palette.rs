//! Command palette overlay (`:` to open).
//!
//! Mirrors the TUI palette in `src/ui/command_palette.rs` — same
//! state struct (`AppState::palette`), same fuzzy-matched entry list,
//! same `Enter` / `Esc` / `Up` / `Down` keystroke routing. Iced
//! renders the overlay; key events flow through the GUI's
//! `handle_key_press` which forwards them to
//! `crate::ui::command_palette::handle_key` whenever
//! `state.palette.open` is true.
//!
//! Display-only widget — every keystroke is intercepted in
//! `App::handle_key_press` so the iced text widget never owns
//! keyboard focus. The query rendered here is `state.palette.query`,
//! which the shared palette key handler updates via `tui_input`
//! semantics. The selected row is highlighted from
//! `state.palette.selected`.

use iced::widget::{column, container, row, scrollable, Column, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::AppState;
use crate::ui_gui::message::GuiMessage;

use crate::ui_gui::widgets::text;
const POPUP_WIDTH: f32 = 720.0;
const RESULTS_HEIGHT: f32 = 480.0;

pub fn view(state: &AppState) -> Element<'_, GuiMessage> {
    // Header: prompt indicator + the live query string. Looks like
    // the TUI palette's "`: query|`" prompt.
    let prompt = text(":").size(20);
    let query_text = text(state.palette.query.clone()).size(18);
    let header = row![
        prompt,
        Space::with_width(8),
        query_text,
        Space::with_width(Length::Fill),
    ]
    .align_y(Alignment::Center)
    .padding([6, 12]);

    // Result list: walk `state.palette.matches` (indices into
    // `state.palette.entries`) so the order respects fuzzy ranking.
    let mut rows: Vec<Element<'_, GuiMessage>> = Vec::new();
    let selected = state.palette.selected;
    for (visual_idx, &entry_idx) in state.palette.matches.iter().enumerate() {
        let Some(entry) = state.palette.entries.get(entry_idx) else { continue };
        let is_selected = visual_idx == selected;
        let label_text = text(entry.label.clone()).size(15);
        let hint_text = text(entry.hint.clone()).size(13);
        let row_body = row![
            label_text,
            Space::with_width(Length::Fill),
            hint_text,
        ]
        .align_y(Alignment::Center)
        .padding([4, 12]);

        let styled = container(row_body)
            .width(Length::Fill)
            .style(move |theme: &Theme| {
                let p = theme.extended_palette();
                let (bg, fg) = if is_selected {
                    (p.primary.strong.color, p.primary.strong.text)
                } else {
                    (Color::TRANSPARENT, p.background.base.text)
                };
                container::Style {
                    background: Some(Background::Color(bg)),
                    text_color: Some(fg),
                    border: Border::default(),
                    ..container::Style::default()
                }
            });
        rows.push(styled.into());
    }

    let list: Element<'_, GuiMessage> = if rows.is_empty() {
        container(text("(no matches)").size(14))
            .padding(20)
            .center_x(Length::Fill)
            .into()
    } else {
        scrollable(Column::with_children(rows).spacing(0))
            .height(Length::Fixed(RESULTS_HEIGHT))
            .into()
    };

    let body: Element<'_, GuiMessage> = column![
        header,
        container(Space::with_height(1.0))
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(
                    theme.extended_palette().background.strong.color,
                )),
                ..container::Style::default()
            })
            .width(Length::Fill),
        list,
    ]
    .into();

    container(body)
        .width(Length::Fixed(POPUP_WIDTH))
        .style(|theme: &Theme| {
            let p = theme.extended_palette();
            container::Style {
                background: Some(Background::Color(p.background.base.color)),
                border: Border {
                    color: p.background.strong.color,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}
