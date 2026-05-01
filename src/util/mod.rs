//! Utility functions.

pub mod help_text;
mod lockfile;
mod logging;
pub mod paths;
#[cfg(feature = "tui")]
mod terminal;
mod text;

pub use lockfile::{LockError, ProcessLock};
pub use logging::setup_logging;
#[cfg(feature = "tui")]
pub use terminal::{restore_terminal, setup_terminal};
pub use text::{force_text_presentation, format_bytes, format_duration, pad_right, sanitize_display_text, truncate_middle, truncate_str};
