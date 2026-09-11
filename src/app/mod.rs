//! Application core module.
//!
//! Contains state management, event handling, and the main event loop.
//!
//! The `theme` submodule holds UI-agnostic theme identifiers so `AppState`
//! remains decoupled from the rendering layer.

pub mod action;
pub mod dispatch;
pub mod event;
pub mod event_core;
pub mod handlers;

pub mod state;
pub mod theme;

pub use action::Action;
pub use event::Event;
pub use handlers::key_input::{available_alt_commands, AltCommand, CommandModifier};
pub use state::{AppState, BrowseCategory, Focus, PlayStatus, RightPanelMode, View};
pub mod command_palette;
pub mod presentation;
pub mod scrollbar;
pub mod tasks;

pub mod meters;
pub mod sources;
