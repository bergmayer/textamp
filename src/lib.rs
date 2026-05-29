//! textamp - a Plex Music client for the terminal.
//!
//! The `textamp` binary uses ratatui + crossterm. Core modules
//! (`app`, `audio`, `config`, `plex`, `services`, `miller`, `util`)
//! carry no UI-specific types — the `ui` module is the only renderer.

pub mod app;
pub mod audio;
pub mod config;
pub mod miller;
pub mod plex;
pub mod services;
pub mod ui;
pub mod util;
