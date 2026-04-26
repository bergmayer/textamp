//! Application core module.
//!
//! Contains state management, event handling, and the main event loop.
//!
//! The `theme` submodule holds UI-agnostic theme identifiers so `AppState`
//! remains decoupled from any concrete UI (ratatui, iced).

pub mod action;
pub mod dispatch;
pub mod event;
pub mod event_core;
#[cfg(feature = "tui")]
mod event_loop;
pub mod handlers;

pub mod state;
pub mod theme;

pub use action::Action;
pub use event::Event;
#[cfg(feature = "tui")]
pub use event_loop::EventLoop;
pub use state::{AppState, AuthState, AuthStep, ConnectionState, PlayStatus, View, BrowseCategory, Focus, RightPanelMode};
pub use handlers::key_input::{AltCommand, available_alt_commands, CommandModifier};
