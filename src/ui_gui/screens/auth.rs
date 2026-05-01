//! Authentication screen — username/password form, PIN display, server picker.
//!
//! Matches the TUI's AuthStep progression: Checking → Login / AuthPin →
//! Authenticating → ServerSelect → Connecting → Connected.

use iced::widget::{button, column, container, text_input};
use iced::{Alignment, Element, Length};

use crate::app::action::SettingsAction;
use crate::app::{Action, AppState};
use crate::app::state::{AuthStep, ConnectionState};
use crate::ui_gui::message::GuiMessage;

use crate::ui_gui::widgets::text;
pub fn view(state: &AppState) -> Element<'_, GuiMessage> {
    let body: Element<'_, GuiMessage> = match state.auth_state.step {
        AuthStep::Checking => container(text("Checking stored credentials\u{2026}").size(16))
            .padding(24)
            .into(),

        AuthStep::Login => login_form(state),

        AuthStep::Authenticating => container(column![
            text("Signing in\u{2026}").size(18),
            text(state.auth_state.username_input.as_str()).size(14),
        ].spacing(8))
            .padding(24)
            .into(),

        AuthStep::ServerSelect => server_picker(state),

        AuthStep::Connecting => {
            let label = match &state.connection {
                ConnectionState::AuthPending { pin_code, .. } => {
                    format!("Open plex.tv/link and enter PIN: {pin_code}")
                }
                _ => "Connecting to server\u{2026}".to_string(),
            };
            container(text(label).size(16)).padding(24).into()
        }
    };

    container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

fn login_form(state: &AppState) -> Element<'_, GuiMessage> {
    // `AuthSignIn` is the action that reads from `state.auth_state.*`
    // (which is what the login form's text_inputs bind to). The similar
    // `SettingsSignIn` reads `state.settings_state.*` — that path is
    // wired to the Settings screen's account sub-form, not this one.
    let submit_action = GuiMessage::Action(Action::Settings(SettingsAction::AuthSignIn));

    // `on_input` makes the inputs editable — without it the text_input
    // renders but never sends key events to the app, so typing drops.
    // `on_submit` lets Enter submit the form from either field.
    let username = text_input("Plex username or email", &state.auth_state.username_input)
        .on_input(GuiMessage::AuthUsernameChanged)
        .on_submit(submit_action.clone())
        .padding(8)
        .size(16);
    let password = text_input("Password", &state.auth_state.password_input)
        .on_input(GuiMessage::AuthPasswordChanged)
        .on_submit(submit_action.clone())
        .secure(true)
        .padding(8)
        .size(16);
    let sign_in = button(text("Sign in").size(16))
        .on_press(submit_action)
        .padding([6, 18]);

    let error = state.auth_state.error_message.as_deref().unwrap_or("");

    // The outer container fills the view and centers the form both ways.
    container(
        column![
            text("Sign in to Plex").size(24),
            username,
            password,
            sign_in,
            text(error).size(14),
        ]
        .spacing(12)
        .align_x(Alignment::Center)
        .max_width(380),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .padding(24)
    .into()
}

fn server_picker(state: &AppState) -> Element<'_, GuiMessage> {
    let rows = state.available_servers.iter().enumerate().map(|(i, s)| {
        let label = format!(
            "{} {}  \u{2014} {} connections",
            if i == state.auth_state.server_index { "\u{25B8}" } else { " " },
            s.name,
            s.connections.len(),
        );
        button(text(label).size(15))
            .width(Length::Fill)
            .on_press(GuiMessage::Action(Action::Settings(SettingsAction::SelectServer(s.client_identifier.clone()))))
            .padding([3, 8])
            .into()
    }).collect::<Vec<Element<'_, GuiMessage>>>();

    container(
        column![
            text("Select a Plex server").size(20),
            iced::widget::Column::with_children(rows).spacing(2),
        ]
        .spacing(8)
        .max_width(540),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .padding(24)
    .into()
}
