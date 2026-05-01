//! Modal popups.
//!
//! Wraps the base view in an overlay-like stack when any popup is active.
//! Popups are rendered on top of a darkened body.

use iced::widget::{container, mouse_area, stack};
use iced::{Element, Length};

use crate::app::AppState;
use crate::ui_gui::message::GuiMessage;

pub mod about;
mod sort;
mod library_picker;
mod bio;
mod dialogs;
mod search;
mod radio_launcher;
mod artist_radio_picker;
mod adventure_launcher;
pub mod command_palette;
pub mod stations;
pub mod similar;
pub mod related;
pub mod settings_popup;
pub mod dj_modes;
pub mod remix_tools;
pub mod user_guide;
pub mod keyboard_shortcuts;

/// Wrap `base` with any active popup overlay. Three layers are
/// stacked so click handling is unambiguous:
///
/// 1. base — the regular app view, behind the dim.
/// 2. blocker — a full-screen dimmed `mouse_area` that captures any
///    click landing OUTSIDE the popup (Noop), so the base never sees
///    it. This is what kept stealing focus before the rework.
/// 3. popup — the actual popup widgets, centred. Their button /
///    text-input / scroll handlers receive clicks normally because
///    the popup sits ABOVE the blocker in the stack and Iced's
///    `Stack::on_event` polls top-down, capturing on the first hit.
///
/// The previous implementation wrapped the popup INSIDE the blocker
/// `mouse_area`, which meant the blocker's own `on_press` handler
/// fired on every click (including ones aimed at popup buttons).
/// `mouse_area::on_event` now happens to delegate to its child first
/// and short-circuit on Captured, so that ordering would technically
/// work — but separating blocker and popup into sibling z-layers
/// makes the intent obvious and tolerates future iced changes.
pub fn overlay<'a>(state: &'a AppState, base: Element<'a, GuiMessage>) -> Element<'a, GuiMessage> {
    let popup = active_popup(state);
    match popup {
        None => base,
        Some(inner) => {
            let dim_bg: Element<'a, GuiMessage> = container(iced::widget::Space::new(Length::Fill, Length::Fill))
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_theme: &iced::Theme| container::Style {
                    background: Some(iced::Background::Color(iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.55 })),
                    ..container::Style::default()
                })
                .into();
            // Catch every mouse-button event so nothing leaks to the
            // base view behind the dim. Each popup has its own Close
            // affordance so outside-clicks intentionally do nothing.
            let blocker: Element<'a, GuiMessage> = mouse_area(dim_bg)
                .on_press(GuiMessage::Noop)
                .on_release(GuiMessage::Noop)
                .on_right_press(GuiMessage::Noop)
                .on_right_release(GuiMessage::Noop)
                .on_middle_press(GuiMessage::Noop)
                .on_middle_release(GuiMessage::Noop)
                .into();
            let centered_popup: Element<'a, GuiMessage> = container(inner)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .into();
            stack![base, blocker, centered_popup].into()
        }
    }
}

fn active_popup(state: &AppState) -> Option<Element<'_, GuiMessage>> {
    // Command palette outranks every other popup: it's a transient
    // overlay opened with `:` and dismissed by Esc / Enter, and the
    // user can fire any other action from inside it. Match the TUI
    // event loop's "palette swallows every key while open" rule.
    if state.palette.open {
        return Some(command_palette::view(state));
    }
    // Order matches the TUI's hit-test precedence: dialogs outrank list popups.
    if let Some(d) = state.popups.confirm_dialog.as_ref() {
        return Some(dialogs::confirm(d));
    }
    if let Some(d) = state.popups.input_dialog.as_ref() {
        return Some(dialogs::input(d));
    }
    if state.popups.library_picker_active {
        return Some(library_picker::view(state));
    }
    if state.popups.search_active {
        return Some(search::view(state));
    }
    if let Some(bio) = state.popups.artist_bio.as_ref() {
        return Some(bio::view(bio));
    }
    if let Some(sort) = state.popups.sort.as_ref() {
        return Some(sort::view(sort));
    }
    if let Some(rl) = state.popups.radio_launcher.as_ref() {
        return Some(radio_launcher::view(rl));
    }
    if let Some(arp) = state.popups.artist_radio_picker.as_ref() {
        return Some(artist_radio_picker::view(arp));
    }
    if let Some(al) = state.popups.adventure_launcher.as_ref() {
        return Some(adventure_launcher::view(al));
    }
    None
}
