//! User interface module.
//!
//! Pure functional rendering using Ratatui.

mod app;
pub mod artwork;
pub mod hit_regions;
pub mod layout;
pub mod screens;
pub mod theme;
pub mod widgets;

pub use app::render;
pub use app::confirm_dialog_hit_test;
pub use app::{init_bio_artwork_renderer, set_bio_artwork_mode, set_bio_artwork_protocol_type, restore_bio_artwork_native_protocol};
pub use artwork::ArtworkRenderer;
pub use theme::{Theme, ThemeName, set_theme, theme};
