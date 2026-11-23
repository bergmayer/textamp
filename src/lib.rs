//! textamp - a music player for the terminal.
//!
//! The `textamp` binary uses ratatui + crossterm. The app module owns state
//! and orchestration, including terminal input adapters. The ui module renders
//! it; audio, config, server access, and services provide the underlying operations.

pub mod app;
pub mod audio;
pub mod audiomuse;
pub mod config;
pub mod icon;
pub mod library;
pub mod media;
pub mod miller;
pub mod services;
pub mod ui;
pub mod util;

pub mod navidrome;
pub mod tui;

#[cfg(feature = "desktop")]
pub mod desktop;

#[cfg(all(feature = "desktop", target_os = "macos"))]
pub mod text_mode;

pub mod terminal;

#[cfg(feature = "mobile")]
pub mod mobile;
