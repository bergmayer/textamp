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
pub use state::{AppState, AuthState, AuthStep, ConnectionState, PlayStatus, RepeatMode, View, BrowseCategory, Focus, RightPanelMode};
