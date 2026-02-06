//! Handler modules for the event loop.
//!
//! Each module contains free functions that receive a `HandlerContext` (or relevant
//! subset of parameters) instead of `&self`. This allows splitting the monolithic
//! event loop into focused, maintainable modules.

pub mod context;
pub mod events;
pub mod helpers;
pub mod key_input;
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
