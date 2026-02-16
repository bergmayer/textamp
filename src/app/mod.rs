//! Application core module.
//!
//! Contains state management, event handling, and the main event loop.

mod action;
mod event;
mod event_loop;
mod handlers;

pub mod state;

pub use action::Action;
pub use event::Event;
pub use event_loop::EventLoop;
pub use state::{AppState, AuthState, AuthStep, ConnectionState, PlayStatus, View, BrowseCategory, Focus, RightPanelMode};
pub use handlers::key_input::{AltCommand, available_alt_commands, CommandModifier};
