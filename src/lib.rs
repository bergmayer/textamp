//! textamp - a Plex Music client.
//!
//! This crate ships two front-ends that share a single core:
//! - `textamp` (binary, requires `feature = "tui"`): ratatui + crossterm TUI.
//! - `textamp-gui` (binary, requires `feature = "gui"`): iced + muda desktop GUI.
//!
//! Core modules (`app`, `audio`, `config`, `plex`, `services`, `miller`,
//! `util`) are always compiled and carry no UI-specific types. The `ui`
//! module (ratatui) and `ui_gui` module (iced) are compiled behind the
//! matching feature flag.

pub mod app;
pub mod audio;
pub mod config;
pub mod miller;
pub mod plex;
pub mod services;
pub mod util;

#[cfg(feature = "tui")]
pub mod ui;

#[cfg(feature = "gui")]
pub mod ui_gui;
