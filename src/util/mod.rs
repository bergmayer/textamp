//! Utility functions.

mod logging;
mod terminal;
mod text;

pub use logging::setup_logging;
pub use terminal::{restore_terminal, setup_terminal};
pub use text::{format_bytes, format_duration, truncate_str};
