//! User interface module.
//!
//! Pure functional rendering using Ratatui.

mod app;
pub(crate) mod artist_links;
pub mod artwork;
pub mod command_palette;
pub(crate) mod detail_links;
mod fullscreen;
pub mod layout;
pub(crate) mod motion;
pub mod screens;
pub mod theme;
pub mod widgets;

pub use app::confirm_dialog_hit_test;
pub use app::{
    init_bio_artwork_renderer, restore_bio_artwork_native_protocol, set_bio_artwork_mode,
    set_bio_artwork_protocol_type,
};
pub use artwork::ArtworkRenderer;
pub use theme::{Theme, ThemeName, set_theme, theme};

use crate::app::{
    AppState,
    presentation::{HitRegions, RenderFeedback},
    state::MarqueeState,
};
use std::cell::RefCell;

/// Read-only application input plus render-local output. Deref only exposes
/// immutable application state; there is intentionally no DerefMut.
pub struct RenderState<'a> {
    app: &'a AppState,
    pub hit_regions: RefCell<HitRegions>,
    pub overlays: bool,
    host: OutputHost,
    pub marquee: RefCell<MarqueeState>,
    pub marquee_subtitle: RefCell<MarqueeState>,
}
impl RenderState<'_> {
    pub(crate) fn cell_width(&self) -> f32 {
        match self.host {
            OutputHost::Terminal => 10.,
            #[cfg(all(feature = "desktop", target_os = "macos"))]
            OutputHost::Embedded {
                cell_width,
                cell_height,
                ..
            } => f32::from(cell_width) / f32::from(cell_height.max(1)) * 20.,
        }
    }
    pub(crate) fn sidebar_width(&self) -> u16 {
        let width = self.window_geometry().sidebar_width;
        (if width > 0 {
            f32::from(width)
        } else {
            layout::SIDEBAR_WIDTH
        } / self.cell_width())
        .round() as u16
    }
    pub(crate) fn window_geometry(&self) -> layout::WindowGeometry {
        match self.host {
            OutputHost::Terminal => layout::WindowGeometry::default(),
            #[cfg(all(feature = "desktop", target_os = "macos"))]
            OutputHost::Embedded { geometry, .. } => geometry,
        }
    }
    pub(crate) fn queue_width(&self) -> u16 {
        let width = self.window_geometry().queue_width;
        (if width > 0 {
            f32::from(width)
        } else {
            layout::SIDEBAR_WIDTH
        } / self.cell_width())
        .round() as u16
    }
    pub(crate) fn graphical_visualizer(&self) -> bool {
        self.host != OutputHost::Terminal
            && self.artwork.mode == crate::app::state::ArtworkMode::Auto
            && matches!(self.popups.modal(), crate::app::state::Modal::Closed)
            && !self.palette.open
            && self.view != crate::app::state::View::Settings
    }
}

impl std::ops::Deref for RenderState<'_> {
    type Target = AppState;
    fn deref(&self) -> &AppState {
        self.app
    }
}

/// Graphics capability belongs to the output adapter, including its first prepared frame.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputHost {
    Terminal,
    #[cfg(all(feature = "desktop", target_os = "macos"))]
    Embedded {
        cell_width: u16,
        cell_height: u16,
        geometry: layout::WindowGeometry,
    },
}
thread_local! {
    static PREVIOUS_APPEARANCE: RefCell<Option<(crate::app::state::ArtworkMode,crate::app::state::View)>> = const {RefCell::new(None)};
}
/// Invalidate renderer-local appearance after replacing its graphics encoder.
#[cfg(all(feature = "desktop", target_os = "macos"))]
pub(crate) fn reset_artwork_presentation() {
    PREVIOUS_APPEARANCE.with(|previous| *previous.borrow_mut() = None);
}

pub fn render(frame: &mut ratatui::Frame, state: &AppState) -> RenderFeedback {
    render_inner(frame, state, None, OutputHost::Terminal)
}

pub(crate) fn render_with_motion(
    frame: &mut ratatui::Frame,
    state: &AppState,
    motion: &mut motion::SceneMotion,
    host: OutputHost,
) -> RenderFeedback {
    render_inner(frame, state, Some(motion), host)
}

fn render_inner(
    frame: &mut ratatui::Frame,
    state: &AppState,
    mut motion: Option<&mut motion::SceneMotion>,
    host: OutputHost,
) -> RenderFeedback {
    let now = std::time::Instant::now();
    if let Some(motion) = &mut motion {
        motion.prepare(state, now);
    }
    sync_appearance(
        state,
        motion.as_ref().is_some_and(|motion| motion.active(now)),
        host,
    );
    let mut input = RenderState {
        app: state,
        hit_regions: RefCell::new(HitRegions::default()),
        overlays: true,
        host,
        marquee: RefCell::new(state.marquee.clone()),
        marquee_subtitle: RefCell::new(state.marquee_subtitle.clone()),
    };
    if let Some(motion) = motion.as_mut().filter(|motion| motion.needs_backdrop()) {
        input.overlays = false;
        app::render(frame, &input);
        motion.backdrop(frame.buffer_mut());
        input.overlays = true;
    }
    app::render(frame, &input);
    let mut hit_regions = input.hit_regions.into_inner();
    if let Some(motion) = motion {
        motion.compose(frame.buffer_mut(), &mut hit_regions, now);
    }
    if hit_regions.text_input.as_ref().is_some_and(|input| {
        hit_regions
            .motion_hidden
            .iter()
            .any(|hidden| !input.area.intersection(*hidden).is_empty())
    }) {
        hit_regions.text_input = None;
    }
    RenderFeedback {
        hit_regions,
        marquee: input.marquee.into_inner(),
        marquee_subtitle: input.marquee_subtitle.into_inner(),
    }
}

fn sync_appearance(state: &AppState, scene_active: bool, host: OutputHost) {
    use crate::app::state::ArtworkMode;
    theme::set_theme(state.theme);
    let mode = effective_artwork_mode(state, scene_active, host);
    PREVIOUS_APPEARANCE.with(|previous| {
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

/// Resolve artwork encoding against the output adapter's clipping capabilities.
pub(crate) fn effective_artwork_mode(
    state: &AppState,
    scene_active: bool,
    host: OutputHost,
) -> crate::app::state::ArtworkMode {
    use crate::app::state::ArtworkMode;
    let overlay = state.view == crate::app::state::View::Settings
        || !matches!(state.popups.modal(), crate::app::state::Modal::Closed)
        || state.palette.open;
    if state.artwork.mode == ArtworkMode::Auto
        && host == OutputHost::Terminal
        && (scene_active
            || state.tui_motion.active(std::time::Instant::now())
            || overlay
            || !artwork::native_images_available())
    {
        ArtworkMode::Halfblocks
    } else {
        state.artwork.mode
    }
}

#[cfg(test)]
mod artwork_fallback_tests {
    use super::*;
    use crate::app::state::ArtworkMode;
    #[test]
    fn image_preference_falls_back_for_the_terminal_without_being_modified() {
        artwork::init_grid_renderer(ratatui_image::picker::Picker::halfblocks());
        let mut state = AppState::new();
        assert_eq!(
            effective_artwork_mode(&state, false, OutputHost::Terminal),
            ArtworkMode::Halfblocks
        );
        assert_eq!(state.artwork.mode, ArtworkMode::Auto);
        #[cfg(all(feature = "desktop", target_os = "macos"))]
        assert_eq!(
            effective_artwork_mode(
                &state,
                false,
                OutputHost::Embedded {
                    cell_width: 10,
                    cell_height: 20,
                    geometry: Default::default()
                }
            ),
            ArtworkMode::Auto
        );
        state.artwork.mode = ArtworkMode::Braille;
        assert_eq!(
            effective_artwork_mode(&state, false, OutputHost::Terminal),
            ArtworkMode::Braille
        );
    }
}
