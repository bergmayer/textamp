//! Right-click context menu.
//!
//! The menu is a GUI-only affordance (no shared state change — the TUI
//! reaches the same actions via keyboard shortcuts). Each entry bundles a
//! label with the `Vec<Action>` that runs when the user clicks it; the
//! menu closes on any click or when the user presses Escape.
//!
//! Positioning uses absolute padding on a `Length::Fill` container so the
//! menu can appear at the cursor point without a dedicated Iced overlay
//! widget (Iced 0.13 has no built-in context-menu widget).

use iced::widget::{button, column as iced_column, container, mouse_area, Column};
use iced::{Background, Border, Color, Element, Length, Padding, Shadow, Theme, Vector};

use crate::app::Action;
use crate::ui_gui::message::GuiMessage;

use crate::ui_gui::widgets::text;
/// Entry shown in a context menu. Separator renders as a thin divider
/// line; Entry renders a clickable button that emits either a bundle
/// of `Action`s or a specific `GuiMessage` when the label is chosen.
#[derive(Debug, Clone)]
pub enum Entry {
    Sep,
    Entry { label: String, actions: Vec<Action> },
    /// Emits a specific GuiMessage on click — used for entries that
    /// need more than just dispatching Actions (e.g. opening a popup
    /// that the shared Action system doesn't know about).
    Custom { label: String, message: GuiMessage },
}

/// Transient state held by `App` while a context menu is open.
#[derive(Debug, Clone)]
pub struct ContextMenuState {
    pub x: f32,
    pub y: f32,
    pub entries: Vec<Entry>,
}

const MENU_WIDTH: f32 = 240.0;
const ROW_HEIGHT: f32 = 26.0;

/// Render the menu as an overlay. `viewport_w` / `viewport_h` are Iced's
/// logical (post-scale) viewport dimensions — used to clamp the menu so
/// it stays on screen when the cursor is near the right/bottom edge.
pub fn view<'a>(state: &'a ContextMenuState, viewport_w: f32, viewport_h: f32) -> Element<'a, GuiMessage> {
    let rows = state
        .entries
        .iter()
        .map(|e| match e {
            Entry::Sep => container(text(" ").size(3))
                .width(Length::Fill)
                .height(Length::Fixed(1.0))
                .style(|theme: &Theme| {
                    let p = theme.extended_palette();
                    container::Style {
                        background: Some(Background::Color(p.background.strong.color)),
                        ..container::Style::default()
                    }
                })
                .into(),
            Entry::Entry { label, actions } => {
                let actions = actions.clone();
                button(text(label.clone()).size(15))
                    .width(Length::Fill)
                    .padding([4, 10])
                    .on_press_with(move || GuiMessage::ContextMenuClick(actions.clone()))
                    .style(entry_btn_style)
                    .into()
            }
            Entry::Custom { label, message } => {
                let msg = message.clone();
                button(text(label.clone()).size(15))
                    .width(Length::Fill)
                    .padding([4, 10])
                    .on_press_with(move || {
                        // Two messages squeezed into one context-menu
                        // click: close the menu + run the custom action.
                        // We approximate by emitting the custom message
                        // directly and relying on `CloseContextMenu`
                        // being emitted by the outer mouse_area dismiss.
                        msg.clone()
                    })
                    .style(entry_btn_style)
                    .into()
            }
        })
        .collect::<Vec<Element<'a, GuiMessage>>>();

    // Rough height estimate so we can flip the menu up when it would
    // otherwise run off the bottom of the window.
    let est_h = state
        .entries
        .iter()
        .map(|e| match e { Entry::Sep => 1.0, _ => ROW_HEIGHT })
        .sum::<f32>()
        + 12.0;

    let menu_x = state.x.min((viewport_w - MENU_WIDTH).max(0.0));
    let menu_y = if state.y + est_h > viewport_h {
        (state.y - est_h).max(0.0)
    } else {
        state.y
    };

    let menu = container(iced_column![
        Column::with_children(rows).spacing(0),
    ]
    .spacing(0))
    .width(Length::Fixed(MENU_WIDTH))
    .padding(4)
    .style(|theme: &Theme| {
        let p = theme.extended_palette();
        container::Style {
            background: Some(Background::Color(p.background.base.color)),
            text_color: Some(p.background.base.text),
            border: Border {
                color: p.primary.strong.color,
                width: 1.0,
                radius: 4.0.into(),
            },
            shadow: Shadow {
                color: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.4 },
                offset: Vector::new(1.0, 2.0),
                blur_radius: 6.0,
            },
        }
    });

    // Transparent full-screen mouse_area around the menu catches clicks
    // outside to dismiss. The menu itself sits inside via padding (this
    // is how we position without a dedicated overlay widget).
    let positioned = container(menu)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: menu_y,
            right: 0.0,
            bottom: 0.0,
            left: menu_x,
        });

    mouse_area(positioned)
        .on_press(GuiMessage::CloseContextMenu)
        .on_right_press(GuiMessage::CloseContextMenu)
        .into()
}

fn entry_btn_style(theme: &Theme, status: button::Status) -> button::Style {
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
}
