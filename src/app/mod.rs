//! Application core module.
//!
//! Contains state management, event handling, and the main event loop.
//!
//! The `theme` submodule holds UI-agnostic theme identifiers so `AppState`
//! remains decoupled from the rendering layer.

pub mod action;
pub mod activity;
pub mod catalog_loading;
pub mod dispatch;
pub mod event;
pub mod event_core;
pub mod external_search;
pub mod handlers;

pub mod fullscreen;
pub mod library_details;
pub mod lyrics;
pub mod navigation_history;
pub mod result_details;
pub mod sonic_details;
pub mod state;
pub mod theme;
pub mod theme_palette;

pub use action::Action;
pub use event::Event;
pub use handlers::key_input::{AltCommand, CommandModifier, available_alt_commands};
pub use state::{AppState, BrowseCategory, Focus, PlayStatus, RightPanelMode, View};
pub mod command_catalog;
pub mod command_history;
pub mod command_palette;
pub mod presentation;
pub mod scrollbar;
pub mod shortcuts;
pub mod tasks;

pub mod meters;
pub mod sources;

pub mod runtime;

pub mod artwork_mosaic;
pub mod browse_drill;
pub mod filtering;
pub mod track_context;

pub mod album_collections;

pub mod sidebar;

pub mod column_controls;

pub mod journey;

pub mod transport_controls;

/// Terminal pane traversal and contextual control descriptions.
pub mod tui_navigation;

/// Cell-based terminal panel transitions.
pub mod tui_motion;

/// Shared preference definitions for all frontends.
pub mod preferences;

/// Shared playback command variants.
pub mod playback_controls;

/// Shared queue occurrence and reorder policy.
pub mod queue_edit;

/// Shared queue-tool semantics and availability.
pub mod queue_controls;

/// Shared details-pane semantics.
pub mod details;

/// Shared column view options.
pub mod view_options;

pub mod launch;
pub mod windowed_presentation;

/// Literal text entry for terminal adapters.
pub mod terminal_text;

/// Stable viewport anchors for presentation transitions.
pub mod view_continuity;

/// Presentation-independent Settings drafts and location.
pub mod settings_transfer;

#[cfg(feature = "desktop")]
pub(crate) mod workflow;

pub mod downloads;

pub mod queue_pointer;

pub mod queue_modes;

pub mod radio_navigation;

pub mod player_device_settings;
pub mod player_sync;
