//! Utility functions.

mod logging;
pub mod paths;
mod terminal;
mod text;

pub use logging::setup_logging;
pub use terminal::{restore_terminal, setup_terminal};
pub use text::{format_bytes, format_duration, pad_right, truncate_middle, truncate_str};
