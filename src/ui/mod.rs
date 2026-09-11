//! User interface module.
//!
//! Pure functional rendering using Ratatui.

mod app;
pub mod artwork;
pub mod command_palette;
pub mod layout;
pub mod screens;
pub mod theme;
pub mod widgets;

pub use app::confirm_dialog_hit_test;
pub use app::{
    init_bio_artwork_renderer, restore_bio_artwork_native_protocol, set_bio_artwork_mode,
    set_bio_artwork_protocol_type,
};
pub use artwork::ArtworkRenderer;
pub use theme::{set_theme, theme, Theme, ThemeName};

use crate::app::{
    presentation::{HitRegions, RenderFeedback},
    state::MarqueeState,
    AppState,
};
use std::cell::RefCell;

/// Read-only application input plus render-local output. Deref only exposes
/// immutable application state; there is intentionally no DerefMut.
pub struct RenderState<'a> {
    app: &'a AppState,
    pub hit_regions: RefCell<HitRegions>,
    pub marquee: RefCell<MarqueeState>,
    pub marquee_subtitle: RefCell<MarqueeState>,
}
impl std::ops::Deref for RenderState<'_> {
    type Target = AppState;
    fn deref(&self) -> &AppState {
        self.app
    }
}

pub fn render(frame: &mut ratatui::Frame, state: &AppState) -> RenderFeedback {
    sync_appearance(state);
    let input = RenderState {
        app: state,
        hit_regions: RefCell::new(HitRegions::default()),
        marquee: RefCell::new(state.marquee.clone()),
        marquee_subtitle: RefCell::new(state.marquee_subtitle.clone()),
    };
    app::render(frame, &input);
    RenderFeedback {
        hit_regions: input.hit_regions.into_inner(),
        marquee: input.marquee.into_inner(),
        marquee_subtitle: input.marquee_subtitle.into_inner(),
    }
}

fn sync_appearance(state: &AppState) {
    use crate::app::state::{ArtworkMode, View};
    thread_local! {
        static PREVIOUS: RefCell<Option<(ArtworkMode, View)>> = const { RefCell::new(None) };
    }
    theme::set_theme(state.theme);
    let mode = if state.artwork.mode == ArtworkMode::Auto
        && (std::env::var("TERM_PROGRAM").as_deref() == Ok("Apple_Terminal")
            || std::env::var("TERM_SESSION_ID").is_ok_and(|id| id.contains("com.apple.Terminal")))
    {
        ArtworkMode::Braille
    } else {
        state.artwork.mode
    };
    PREVIOUS.with(|previous| {
        let old = previous.replace(Some((mode, state.view)));
        if old.is_none_or(|(old_mode, _)| old_mode != mode) {
            screens::now_playing::set_artwork_mode(mode);
            artwork::set_grid_artwork_mode(mode);
            set_bio_artwork_mode(mode);
            match mode {
                ArtworkMode::Halfblocks => {
                    let protocol = ratatui_image::picker::ProtocolType::Halfblocks;
                    screens::now_playing::set_artwork_protocol_type(protocol);
                    artwork::set_grid_protocol_type(protocol);
                    set_bio_artwork_protocol_type(protocol);
                }
                ArtworkMode::Auto => {
                    screens::now_playing::restore_artwork_native_protocol();
                    artwork::restore_grid_native_protocol();
                    restore_bio_artwork_native_protocol();
                }
                ArtworkMode::Braille => {}
            }
        }
        if old.is_some_and(|(_, view)| view != state.view) {
            screens::now_playing::clear_artwork_cache();
        }
    });
}
