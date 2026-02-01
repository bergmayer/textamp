//! Library management service.
//!
//! Handles library selection and default library settings.

use crate::config::Config;

/// Service for library management.
pub struct LibraryService;

impl LibraryService {
    /// Get the default library key, if set.
    pub fn get_default_library(config: &Config) -> Option<&str> {
        config.libraries.default_library.as_deref()
    }

    /// Set the default library key.
    pub fn set_default_library(config: &mut Config, library_key: Option<String>) {
        config.libraries.default_library = library_key;
    }
}
