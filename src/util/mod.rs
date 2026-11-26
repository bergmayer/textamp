//! Utility functions.

pub(crate) mod column_heading;
pub mod help_text;
mod lockfile;
mod logging;
pub mod paths;
mod secret;
mod terminal;
mod text;

pub use lockfile::{LockError, ProcessLock};
pub use logging::{activity_error, activity_history_text, setup_logging};
pub use secret::SecretString;
pub(crate) use terminal::catch_expected_panic;
pub use terminal::{
    AppTerminal, TerminalSession, install_panic_hook, restore_terminal, setup_terminal,
};
pub use text::{
    force_text_presentation, format_bytes, format_duration, pad_right, sanitize_display_text,
    truncate_middle, truncate_str, truncate_to_boundary,
};
pub(crate) mod private_file;
pub(crate) mod serde_helpers;

pub mod server_url;
