//! Track window size so layout-sensitive logic (responsive scale factor,
//! Miller column visibility window, now-playing art sizing) can read the
//! current viewport in pixels.

#[derive(Debug, Clone, Copy, Default)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}
