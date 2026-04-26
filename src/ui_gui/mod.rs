//! Desktop GUI front-end (Iced + muda).
//!
//! Compiled under `feature = "gui"`. Provides a desktop window, a native
//! menu bar, keyboard shortcuts, and rendered views that mirror the TUI
//! feature-set. Drives the same `Action`s through the same dispatch handlers
//! as the TUI so the two front-ends stay in lockstep.

mod app;
mod message;
mod theme;
mod menu;
mod shortcuts;
mod viewport;

pub mod input;
pub mod screens;
pub mod widgets;
pub mod images;

pub use app::run;
