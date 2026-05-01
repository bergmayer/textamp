//! Handler modules for the event loop.
//!
//! Each module contains free functions that receive a `HandlerContext` (or relevant
//! subset of parameters) instead of `&self`. This allows splitting the monolithic
//! event loop into focused, maintainable modules.
//!
//! `key_input` holds both the crossterm-fed dispatcher (TUI) and a set of
//! pure action-builder helpers (e.g. `navigate_to_album`, `get_similar_action`,
//! `truncate_filter_right_columns`) that the `dispatch_*` modules call
//! regardless of UI. It stays always-compiled so the GUI can reuse those
//! helpers without pulling in the full TUI event loop.
//!
//! `mouse_input` is TUI-only (it depends on ratatui-layout hit regions stored
//! on `AppState`) and is gated behind `feature = "tui"`. The GUI has its own
//! mouse handling in `crate::ui_gui::input`.

pub mod context;
pub mod events;
pub mod helpers;
pub mod key_input;
pub mod lazy_art;
#[cfg(feature = "tui")]
pub mod mouse_input;

pub mod dispatch_browse;
pub mod dispatch_data;
pub mod dispatch_folders;
pub mod dispatch_miller;
pub mod dispatch_navigation;
pub mod dispatch_playback;
pub mod dispatch_queue;
pub mod dispatch_radio;
pub mod dispatch_search;
pub mod dispatch_settings;
pub mod dispatch_system;
