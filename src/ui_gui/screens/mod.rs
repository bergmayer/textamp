//! One module per screen — mirrors `crate::ui::screens`.
//!
//! Each module exposes a `view(state) -> Element<GuiMessage>` function.

pub mod auth;
pub mod browse;
pub mod help;
pub mod now_playing;
pub mod popups;
pub mod queue;
pub mod related;
pub mod search;
pub mod settings;
pub mod similar;
