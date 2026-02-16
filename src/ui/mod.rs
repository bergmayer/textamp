//! User interface module.
//!
//! Pure functional rendering using Ratatui.

mod app;
pub mod artwork;
pub mod layout;
pub mod screens;
pub mod theme;
pub mod widgets;

pub use app::render;
pub use app::confirm_dialog_hit_test;
pub use artwork::ArtworkRenderer;
pub use theme::{Theme, ThemeName, set_theme, theme};
