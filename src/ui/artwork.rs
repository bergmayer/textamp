//! Album artwork support using ratatui-image.
//!
//! Displays album art in terminals that support graphics protocols
//! (Kitty, iTerm2, Sixel) or falls back to halfblocks.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use image::DynamicImage;
use ratatui::prelude::*;
use ratatui_image::{Resize, StatefulImage, picker::Picker, protocol::StatefulProtocol};

use crate::app::state::ArtworkMode;

/// Placeholder identity matching the graphical frontend's artwork icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Placeholder {
    Track,
    Album,
    Artist,
}

impl Placeholder {
    fn mask(self) -> &'static [u8] {
        match self {
            Self::Track => include_bytes!("../../assets/artwork/track.png"),
            Self::Album => include_bytes!("../../assets/artwork/album.png"),
            Self::Artist => include_bytes!("../../assets/artwork/artist.png"),
        }
    }
}

/// Render missing artwork through the selected image, half-block or braille encoder.
pub fn render_placeholder(frame: &mut Frame, area: Rect, kind: Placeholder) -> bool {
    let colors = crate::ui::theme::theme().colors;
    let key = format!(
        "placeholder:{kind:?}:{:?}:{:?}",
        colors.fg_muted, colors.bg_secondary
    );
    let data = PLACEHOLDERS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(data) = cache.get(&key) {
            return Some(data.clone());
        }
        let data = match placeholder_png(kind, colors.fg_muted, colors.bg_secondary) {
            Ok(data) => Arc::new(data),
            Err(error) => {
                tracing::warn!(%error, "Could not prepare artwork placeholder");
                return None;
            }
        };
        cache.insert(key.clone(), data.clone());
        Some(data)
    });
    data.is_some_and(|data| render_grid_image(frame, area, &key, &data))
}

fn placeholder_png(
    kind: Placeholder,
    foreground: Color,
    background: Color,
) -> image::ImageResult<Vec<u8>> {
    let rgb = |color| match color {
        Color::Rgb(r, g, b) => [r, g, b],
        Color::White => [255; 3],
        Color::Reset => [128; 3],
        _ => [0; 3],
    };
    let foreground = rgb(foreground);
    let background = rgb(background);
    let mut pixels = image::load_from_memory(kind.mask())?.to_rgba8();
    for pixel in pixels.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        for channel in 0..3 {
            pixel[channel] = ((u32::from(foreground[channel]) * alpha
                + u32::from(background[channel]) * (255 - alpha))
                / 255) as u8;
        }
        pixel[3] = 255;
    }
    let mut output = std::io::Cursor::new(Vec::new());
    pixels.write_to(&mut output, image::ImageFormat::Png)?;
    Ok(output.into_inner())
}

/// Artwork renderer using ratatui-image.
///
/// This struct manages the picker (terminal capability detection) and the
/// current image protocol for rendering.
pub struct ArtworkRenderer {
    picker: Option<Picker>,
    protocol: Option<StatefulProtocol>,
    current_thumb: Option<String>,
    current_image_id: Option<u64>,
    braille_image: Option<Arc<DynamicImage>>,
    mode: ArtworkMode,
    /// The protocol type detected at startup (for restoring after Halfblocks override).
    native_protocol: Option<ratatui_image::picker::ProtocolType>,
}

impl Default for ArtworkRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtworkRenderer {
    /// Create without a picker (no graphics, placeholder fallback).
    pub fn new() -> Self {
        Self {
            picker: None,
            protocol: None,
            current_thumb: None,
            current_image_id: None,
            braille_image: None,
            mode: ArtworkMode::Auto,
            native_protocol: None,
        }
    }

    /// Create with a pre-initialized picker for graphics support.
    pub fn new_with_picker(picker: Picker) -> Self {
        let native = picker.protocol_type();
        tracing::info!("Artwork protocol: {:?}", native);
        Self {
            picker: Some(picker),
            protocol: None,
            current_thumb: None,
            current_image_id: None,
            braille_image: None,
            mode: ArtworkMode::Auto,
            native_protocol: Some(native),
        }
    }

    /// Get the detected protocol type name (for display in settings).
    pub fn protocol_name(&self) -> &'static str {
        match &self.picker {
            Some(picker) => match picker.protocol_type() {
                ratatui_image::picker::ProtocolType::Halfblocks => "halfblocks",
                ratatui_image::picker::ProtocolType::Sixel => "sixel",
                ratatui_image::picker::ProtocolType::Kitty => "kitty",
                ratatui_image::picker::ProtocolType::Iterm2 => "iterm2",
            },
            None => "none",
        }
    }

    /// Load and prepare an image for rendering.
    /// Returns true if the image was loaded successfully.
    pub fn load_image(&mut self, image_data: &[u8], thumb_path: &str) -> bool {
        let image_id = artwork_id(thumb_path, image_data);
        if self.current_image_id == Some(image_id) {
            if self.mode == ArtworkMode::Braille && self.braille_image.is_some() {
                return true;
            }
            if self.mode != ArtworkMode::Braille && self.protocol.is_some() {
                return true;
            }
        }

        let Some(img) = decoded_image(image_id, image_data) else {
            return false;
        };
        self.protocol = None;
        self.braille_image = Some(img.clone());
        if self.mode != ArtworkMode::Braille {
            let Some(picker) = self.picker.as_mut() else {
                return false;
            };
            self.protocol = Some(picker.new_resize_protocol((*img).clone()));
        }
        self.current_thumb = Some(thumb_path.into());
        self.current_image_id = Some(image_id);
        true
    }

    /// Load an image with the top portion cropped (for scrolling).
    /// `crop_fraction` is 0.0..1.0 indicating how much of the top to remove.
    pub fn load_image_cropped(
        &mut self,
        image_data: &[u8],
        thumb_path: &str,
        crop_fraction: f32,
    ) -> bool {
        if crop_fraction <= 0.0 {
            return self.load_image(image_data, thumb_path);
        }

        // Cache key includes crop amount to avoid re-creating protocol unnecessarily
        let crop_key = format!("{}:c{}", thumb_path, (crop_fraction * 100.0) as u32);
        let image_id = artwork_id(&crop_key, image_data);
        if self.current_image_id == Some(image_id) && self.has_image() {
            return true;
        }

        let Some(img) = decoded_image(artwork_id(thumb_path, image_data), image_data) else {
            return false;
        };

        let crop_pixels =
            (img.height() as f32 * crop_fraction).min(img.height() as f32 - 1.0) as u32;
        let cropped = if crop_pixels > 0 && crop_pixels < img.height() {
            img.crop_imm(0, crop_pixels, img.width(), img.height() - crop_pixels)
        } else {
            (*img).clone()
        };

        if self.mode == ArtworkMode::Braille {
            self.braille_image = Some(Arc::new(cropped));
            self.protocol = None;
            self.current_thumb = Some(crop_key);
            self.current_image_id = Some(image_id);
            return true;
        }

        let Some(ref mut picker) = self.picker else {
            return false;
        };
        self.protocol = Some(picker.new_resize_protocol(cropped));
        self.braille_image = None;
        self.current_thumb = Some(crop_key);
        self.current_image_id = Some(image_id);
        true
    }

    /// Clear the current image.
    pub fn clear(&mut self) {
        self.protocol = None;
        self.braille_image = None;
        self.current_thumb = None;
        self.current_image_id = None;
    }

    /// Render the artwork to a frame area.
    ///
    /// `Resize::Scale` rather than `Resize::Crop`/`Resize::Fit` because
    /// in ratatui-image 9 only `Scale` actually upscales when the
    /// source image is smaller than the render area — the other two
    /// cap at source pixel size and leave a tiny cover drifting in the
    /// upper-left of a much larger box. server thumbs come back at
    /// 300–600 px while the now-playing panel can easily be 800+ px
    /// tall on a wide terminal, so without `Scale` the cover doesn't
    /// fill its square.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        if self.mode == ArtworkMode::Braille {
            if let Some(ref img) = self.braille_image {
                render_braille_image(frame, img, area, self.current_image_id.unwrap_or(0));
            }
            return;
        }
        if let Some(ref mut protocol) = self.protocol {
            let image = StatefulImage::new().resize(Resize::Scale(None));
            frame.render_stateful_widget(image, area, protocol);
        }
    }

    /// Check if there's an image loaded.
    pub fn has_image(&self) -> bool {
        if self.mode == ArtworkMode::Braille {
            return self.braille_image.is_some();
        }
        self.protocol.is_some()
    }

    /// Get the current thumb path.
    pub fn current_thumb(&self) -> Option<&str> {
        self.current_thumb.as_deref()
    }

    /// Change protocol only when the terminal encoder changes.
    pub fn set_protocol_type(&mut self, protocol_type: ratatui_image::picker::ProtocolType) {
        if let Some(picker) = self.picker.as_mut()
            && picker.protocol_type() != protocol_type
        {
            picker.set_protocol_type(protocol_type);
            self.protocol = None;
        }
    }
    /// Restore the terminal's native graphics encoder.
    pub fn restore_native_protocol(&mut self) {
        if let Some(native) = self.native_protocol {
            self.set_protocol_type(native);
        }
    }
    /// Select character or pixel artwork without discarding decoded pixels.
    pub fn set_mode(&mut self, mode: ArtworkMode) {
        self.mode = mode;
    }
}

// ============================================================================
// Braille Image Rendering
// ============================================================================

/// Render an image as Braille characters in the given area.
///
/// Uses adaptive thresholding: computes the median luminance of the resized image
/// so that roughly half the dots are lit, producing dense output regardless of
/// whether the source image is dark or bright.
///
/// Each terminal cell maps to a 2x4 pixel block using Unicode Braille patterns (U+2800..U+28FF).
/// Dot bit mapping per cell:
///   Col 0: bits 0,1,2,6 (rows 0-3)
///   Col 1: bits 3,4,5,7 (rows 0-3)
#[derive(Clone, Copy)]
struct BrailleCell {
    symbol: char,
    foreground: Color,
    background: Color,
}

struct BrailleRender {
    width: u16,
    height: u16,
    cells: Vec<BrailleCell>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct BrailleRenderKey {
    image_id: u64,
    max_width: u16,
    max_height: u16,
}

fn artwork_id(key: &str, data: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    data.hash(&mut hasher);
    hasher.finish()
}

fn build_braille_render(
    img: &DynamicImage,
    max_width: u16,
    max_height: u16,
) -> Option<BrailleRender> {
    use image::GenericImageView;

    if max_width == 0 || max_height == 0 {
        return None;
    }

    // Preserve the source aspect ratio and center the result inside `area`.
    // Braille cells contain 2 × 4 pixels.
    let (src_w, src_h) = img.dimensions();
    if src_w == 0 || src_h == 0 {
        return None;
    }
    let area_px_w = max_width as u32 * 2;
    let area_px_h = max_height as u32 * 4;
    let (dest_px_w, dest_px_h) = if src_w * area_px_h <= src_h * area_px_w {
        // Height-limited.
        ((src_w * area_px_h / src_h).max(1), area_px_h)
    } else {
        // Width-limited.
        (area_px_w, (src_h * area_px_w / src_w).max(1))
    };
    let dest_w = ((dest_px_w / 2) as u16).clamp(1, max_width);
    let dest_h = ((dest_px_h / 4) as u16).clamp(1, max_height);
    let pixel_w = dest_w as u32 * 2;
    let pixel_h = dest_h as u32 * 4;

    let resized = img.resize_exact(pixel_w, pixel_h, image::imageops::FilterType::Triangle);

    // Build luminance histogram for adaptive thresholding
    let mut histogram = [0u32; 256];
    let total_pixels = pixel_w * pixel_h;
    for y in 0..pixel_h {
        for x in 0..pixel_w {
            let pixel = resized.get_pixel(x, y);
            let lum =
                (pixel[0] as u32 * 299 + pixel[1] as u32 * 587 + pixel[2] as u32 * 114) / 1000;
            histogram[lum.min(255) as usize] += 1;
        }
    }

    // Find the median luminance — threshold where ~50% of pixels are above
    let target = total_pixels / 2;
    let mut cumulative = 0u32;
    let mut threshold = 128u32;
    for (lum, &count) in histogram.iter().enumerate() {
        cumulative += count;
        if cumulative >= target {
            threshold = lum as u32;
            break;
        }
    }
    // Clamp threshold so we never go fully black or fully white
    threshold = threshold.clamp(15, 240);

    // Braille dot bit positions:
    // (0,0)→bit0  (1,0)→bit3
    // (0,1)→bit1  (1,1)→bit4
    // (0,2)→bit2  (1,2)→bit5
    // (0,3)→bit6  (1,3)→bit7
    let bit_map: [[u8; 4]; 2] = [
        [0, 1, 2, 6], // col 0, rows 0-3
        [3, 4, 5, 7], // col 1, rows 0-3
    ];

    let mut cells = Vec::with_capacity(dest_width_len(dest_w, dest_h));

    for row in 0..dest_h as u32 {
        for col in 0..dest_w as u32 {
            let px = col * 2;
            let py = row * 4;

            let mut pattern: u8 = 0;
            let mut fg_r: u32 = 0;
            let mut fg_g: u32 = 0;
            let mut fg_b: u32 = 0;
            let mut bright_count: u32 = 0;
            let mut bg_r: u32 = 0;
            let mut bg_g: u32 = 0;
            let mut bg_b: u32 = 0;
            let mut dark_count: u32 = 0;

            for dx in 0..2u32 {
                for dy in 0..4u32 {
                    let x = px + dx;
                    let y = py + dy;
                    if x < pixel_w && y < pixel_h {
                        let pixel = resized.get_pixel(x, y);
                        let r = pixel[0] as u32;
                        let g = pixel[1] as u32;
                        let b = pixel[2] as u32;
                        let lum = (r * 299 + g * 587 + b * 114) / 1000;
                        if lum >= threshold {
                            pattern |= 1 << bit_map[dx as usize][dy as usize];
                            fg_r += r;
                            fg_g += g;
                            fg_b += b;
                            bright_count += 1;
                        } else {
                            bg_r += r;
                            bg_g += g;
                            bg_b += b;
                            dark_count += 1;
                        }
                    }
                }
            }

            let ch = char::from_u32(0x2800 + pattern as u32).unwrap_or(' ');
            let fg_color = if let Some(count) = std::num::NonZeroU32::new(bright_count) {
                Color::Rgb(
                    (fg_r / count.get()) as u8,
                    (fg_g / count.get()) as u8,
                    (fg_b / count.get()) as u8,
                )
            } else {
                Color::Rgb(0, 0, 0)
            };
            let bg_color = if let Some(count) = std::num::NonZeroU32::new(dark_count) {
                Color::Rgb(
                    (bg_r / count.get()) as u8,
                    (bg_g / count.get()) as u8,
                    (bg_b / count.get()) as u8,
                )
            } else {
                fg_color
            };

            cells.push(BrailleCell {
                symbol: ch,
                foreground: fg_color,
                background: bg_color,
            });
        }
    }

    Some(BrailleRender {
        width: dest_w,
        height: dest_h,
        cells,
    })
}

fn dest_width_len(width: u16, height: u16) -> usize {
    usize::from(width).saturating_mul(usize::from(height))
}

fn paint_braille_render(frame: &mut Frame, area: Rect, rendered: &BrailleRender) {
    let origin_x = area.x + area.width.saturating_sub(rendered.width) / 2;
    let origin_y = area.y + area.height.saturating_sub(rendered.height) / 2;
    let buffer = frame.buffer_mut();
    for row in 0..rendered.height {
        for column in 0..rendered.width {
            let index = usize::from(row) * usize::from(rendered.width) + usize::from(column);
            let cell = rendered.cells[index];
            buffer[(origin_x + column, origin_y + row)]
                .set_char(cell.symbol)
                .set_fg(cell.foreground)
                .set_bg(cell.background);
        }
    }
}

fn render_braille_image(frame: &mut Frame, img: &DynamicImage, area: Rect, image_id: u64) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let key = BrailleRenderKey {
        image_id,
        max_width: area.width,
        max_height: area.height,
    };
    BRAILLE_RENDERS.with(|renders| {
        let mut renders = renders.borrow_mut();
        if !renders.contains_key(&key) {
            let Some(rendered) = build_braille_render(img, area.width, area.height) else {
                return;
            };
            renders.insert(key, rendered);
        }
        if let Some(rendered) = renders.get(&key) {
            paint_braille_render(frame, area, rendered);
        }
    });
}

// ============================================================================
// Album Art Grid — shared Picker and per-album protocol cache
// ============================================================================

const MAX_RENDERED_ARTWORK_ENTRIES: usize = 64;

struct BoundedCache<K, V> {
    values: HashMap<K, V>,
    order: VecDeque<K>,
    capacity: usize,
}

impl<K: Clone + Eq + Hash, V> BoundedCache<K, V> {
    fn new(capacity: usize) -> Self {
        Self {
            values: HashMap::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    fn contains_key(&self, key: &K) -> bool {
        self.values.contains_key(key)
    }

    fn touch(&mut self, key: &K) {
        if let Some(position) = self.order.iter().position(|candidate| candidate == key) {
            self.order.remove(position);
        }
        self.order.push_back(key.clone());
    }

    fn insert(&mut self, key: K, value: V) {
        if self.values.contains_key(&key) {
            self.values.insert(key.clone(), value);
            self.touch(&key);
            return;
        }
        while self.values.len() >= self.capacity {
            let Some(oldest) = self.order.pop_front() else {
                self.values.clear();
                break;
            };
            self.values.remove(&oldest);
        }
        self.values.insert(key.clone(), value);
        self.order.push_back(key);
    }

    fn get(&mut self, key: &K) -> Option<&V> {
        if !self.values.contains_key(key) {
            return None;
        }
        self.touch(key);
        self.values.get(key)
    }

    fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        if !self.values.contains_key(key) {
            return None;
        }
        self.touch(key);
        self.values.get_mut(key)
    }

    fn clear(&mut self) {
        self.values.clear();
        self.order.clear();
    }
}

thread_local! {
    static PLACEHOLDERS: RefCell<BoundedCache<String, Arc<Vec<u8>>>> = RefCell::new(BoundedCache::new(12));
    static GRID_PICKER: RefCell<Option<Picker>> = const { RefCell::new(None) };
    static GRID_PROTOCOLS: RefCell<BoundedCache<(u64, Rect), StatefulProtocol>> = RefCell::new(BoundedCache::new(MAX_RENDERED_ARTWORK_ENTRIES));
    static DECODED_IMAGES: RefCell<BoundedCache<u64, Arc<DynamicImage>>> = RefCell::new(BoundedCache::new(MAX_RENDERED_ARTWORK_ENTRIES));
    static BRAILLE_RENDERS: RefCell<BoundedCache<BrailleRenderKey, BrailleRender>> = RefCell::new(BoundedCache::new(MAX_RENDERED_ARTWORK_ENTRIES));
    static GRID_ARTWORK_MODE: RefCell<ArtworkMode> = const { RefCell::new(ArtworkMode::Auto) };
    static GRID_NATIVE_PROTOCOL: RefCell<Option<ratatui_image::picker::ProtocolType>> = const { RefCell::new(None) };
}

/// Initialize the grid renderer with a Picker clone.
/// Call once at startup alongside the main artwork renderer init.
pub fn init_grid_renderer(picker: Picker) {
    reset_grid_placements();
    GRID_NATIVE_PROTOCOL.with(|p| *p.borrow_mut() = Some(picker.protocol_type()));
    GRID_PICKER.with(|p| *p.borrow_mut() = Some(picker));
}

/// Render an album cover image to the given area.
/// Decoded pixels are shared; each on-screen placement owns its resize protocol.
/// Returns true if an image was rendered.
pub fn render_grid_image(frame: &mut Frame, area: Rect, key: &str, data: &[u8]) -> bool {
    let mode = GRID_ARTWORK_MODE.with(|m| *m.borrow());

    if mode == ArtworkMode::Braille {
        return render_grid_braille(frame, area, key, data);
    }

    // Protocol-based rendering (Auto/Halfblocks)
    let image_id = artwork_id(key, data);
    let placement = (image_id, area);
    let has_protocol = GRID_PROTOCOLS.with(|protos| protos.borrow().contains_key(&placement));

    if !has_protocol {
        let created = GRID_PICKER.with(|picker_cell| {
            let mut picker_ref = picker_cell.borrow_mut();
            let Some(picker) = picker_ref.as_mut() else {
                return false;
            };
            let Some(img) = decoded_image(image_id, data) else {
                return false;
            };
            let protocol = picker.new_resize_protocol((*img).clone());
            GRID_PROTOCOLS.with(|protos| {
                protos.borrow_mut().insert(placement, protocol);
            });
            true
        });
        if !created {
            return false;
        }
    }

    GRID_PROTOCOLS.with(|protos| {
        let mut map = protos.borrow_mut();
        if let Some(protocol) = map.get_mut(&placement) {
            // Scale thumbnails to fit the render area.
            let image = StatefulImage::new().resize(Resize::Scale(None));
            frame.render_stateful_widget(image, area, protocol);
            true
        } else {
            false
        }
    })
}

fn decoded_image(image_id: u64, data: &[u8]) -> Option<Arc<DynamicImage>> {
    if let Some(image) = DECODED_IMAGES.with(|images| images.borrow_mut().get(&image_id).cloned()) {
        return Some(image);
    }
    let image = Arc::new(image::load_from_memory(data).ok()?);
    DECODED_IMAGES.with(|images| images.borrow_mut().insert(image_id, image.clone()));
    Some(image)
}

/// Render a grid album cover using braille characters.
fn render_grid_braille(frame: &mut Frame, area: Rect, key: &str, data: &[u8]) -> bool {
    let image_id = artwork_id(key, data);
    let Some(image) = decoded_image(image_id, data) else {
        return false;
    };
    render_braille_image(frame, &image, area, image_id);
    true
}

/// Recreate terminal placements while retaining decoded images and character artwork.
pub fn reset_grid_placements() {
    GRID_PROTOCOLS.with(|protocols| protocols.borrow_mut().clear());
}

/// Clear every presentation and decoded grid image on a library change.
pub fn clear_grid_cache() {
    GRID_PROTOCOLS.with(|protos| protos.borrow_mut().clear());
    DECODED_IMAGES.with(|images| images.borrow_mut().clear());
    BRAILLE_RENDERS.with(|renders| renders.borrow_mut().clear());
}

/// Change the graphics encoder while retaining mode-independent decoded artwork.
pub fn set_grid_protocol_type(protocol_type: ratatui_image::picker::ProtocolType) {
    let changed = GRID_PICKER.with(|cell| {
        let mut picker = cell.borrow_mut();
        if let Some(picker) = picker.as_mut()
            && picker.protocol_type() != protocol_type
        {
            picker.set_protocol_type(protocol_type);
            return true;
        }
        false
    });
    if changed {
        reset_grid_placements();
    }
}
/// Whether the external terminal's detected protocol can display image placements.
pub(crate) fn native_images_available() -> bool {
    GRID_NATIVE_PROTOCOL.with(|native| {
        native
            .borrow()
            .is_some_and(|protocol| protocol != ratatui_image::picker::ProtocolType::Halfblocks)
    })
}

#[cfg(any(feature = "desktop", feature = "mobile"))]
fn character_render(data: &[u8], mode: ArtworkMode) -> Option<(u16, u16, Vec<BrailleCell>)> {
    let image = image::load_from_memory(data).ok()?;
    let result = match mode {
        ArtworkMode::Braille => {
            let render = build_braille_render(&image, 48, 24)?;
            (render.width, render.height, render.cells)
        }
        ArtworkMode::Halfblocks => {
            let scaled = image.thumbnail(48, 48).to_rgb8();
            let width = scaled.width() as u16;
            let height = scaled.height().div_ceil(2) as u16;
            let cells = (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let top = scaled.get_pixel(u32::from(x), u32::from(y) * 2).0;
                    let bottom = scaled
                        .get_pixel(
                            u32::from(x),
                            (u32::from(y) * 2 + 1).min(scaled.height() - 1),
                        )
                        .0;
                    BrailleCell {
                        symbol: '▀',
                        foreground: Color::Rgb(top[0], top[1], top[2]),
                        background: Color::Rgb(bottom[0], bottom[1], bottom[2]),
                    }
                })
                .collect();
            (width, height, cells)
        }
        ArtworkMode::Auto => return None,
    };
    Some(result)
}

/// Colored character cells for hosts that draw ANSI artwork without a terminal emulator.
#[cfg(feature = "mobile")]
pub(crate) type RgbCharacterCell = (char, [u8; 3], [u8; 3]);

/// Encode a character grid with explicit RGB foreground and background colors.
#[cfg(feature = "mobile")]
pub(crate) fn character_cells(
    data: &[u8],
    mode: ArtworkMode,
) -> Option<(u16, u16, Vec<RgbCharacterCell>)> {
    let (width, height, cells) = character_render(data, mode)?;
    let cells = cells
        .into_iter()
        .map(|cell| {
            let Color::Rgb(r, g, b) = cell.foreground else {
                return None;
            };
            let Color::Rgb(br, bg, bb) = cell.background else {
                return None;
            };
            Some((cell.symbol, [r, g, b], [br, bg, bb]))
        })
        .collect::<Option<Vec<_>>>()?;
    Some((width, height, cells))
}

/// Encode the character artwork as SVG for a graphical image widget. Braille
/// shares the terminal encoder; half blocks use the same two-color cell geometry.
#[cfg(feature = "desktop")]
pub(crate) fn character_svg(data: &[u8], mode: ArtworkMode) -> Option<Vec<u8>> {
    use std::fmt::Write;
    let (width, height, cells) = character_render(data, mode)?;
    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}">"#,
        width * 10,
        height * 20,
        width * 10,
        height * 20
    );
    for (index, cell) in cells.iter().enumerate() {
        let x = index % usize::from(width) * 10;
        let y = index / usize::from(width) * 20;
        let Color::Rgb(br, bg, bb) = cell.background else {
            return None;
        };
        let Color::Rgb(fr, fg, fb) = cell.foreground else {
            return None;
        };
        write!(
            svg,
            r#"<rect x="{x}" y="{y}" width="10" height="20" fill="rgb({br},{bg},{bb})"/>"#
        )
        .ok()?;
        if mode == ArtworkMode::Halfblocks {
            write!(
                svg,
                r#"<rect x="{x}" y="{y}" width="10" height="10" fill="rgb({fr},{fg},{fb})"/>"#
            )
            .ok()?;
        } else {
            write!(svg, r#"<text x="{x}" y="{}" font-family="monospace" font-size="20" fill="rgb({fr},{fg},{fb})">{}</text>"#, y+17,cell.symbol).ok()?;
        }
    }
    svg.push_str("</svg>");
    Some(svg.into_bytes())
}

/// Restore the detected terminal graphics encoder.
pub fn restore_grid_native_protocol() {
    if let Some(native) = GRID_NATIVE_PROTOCOL.with(|native| *native.borrow()) {
        set_grid_protocol_type(native);
    }
}
/// Choose pixel or character artwork, preserving reusable image and resize caches.
pub fn set_grid_artwork_mode(mode: ArtworkMode) {
    GRID_ARTWORK_MODE.with(|current| *current.borrow_mut() = mode);
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    #[allow(deprecated)]
    fn placeholders_use_the_selected_character_encoder() {
        crate::ui::theme::set_theme(crate::app::theme::ThemeName::SolarizedDark);
        let mut picker = Picker::from_fontsize((8, 16));
        picker.set_protocol_type(ratatui_image::picker::ProtocolType::Halfblocks);
        init_grid_renderer(picker);
        for mode in [ArtworkMode::Halfblocks, ArtworkMode::Braille] {
            set_grid_artwork_mode(mode);
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 20)).unwrap();
            terminal
                .draw(|frame| {
                    for (index, kind) in
                        [Placeholder::Track, Placeholder::Album, Placeholder::Artist]
                            .into_iter()
                            .enumerate()
                    {
                        assert!(render_placeholder(
                            frame,
                            Rect::new(index as u16 * 20, 0, 20, 10),
                            kind
                        ));
                    }
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            for column in 0..3 {
                assert!((column * 20..column * 20 + 20).any(|x| (0..10).any(|y| {
                    buffer[(x, y)].symbol().chars().any(|symbol| match mode {
                        ArtworkMode::Halfblocks => matches!(symbol, '▀' | '▄'),
                        ArtworkMode::Braille => ('\u{2801}'..='\u{28ff}').contains(&symbol),
                        ArtworkMode::Auto => false,
                    })
                })));
            }
        }
    }
    #[test]
    fn artwork_presentation_changes_reuse_decoded_pixels_and_character_renders() {
        let image = DynamicImage::new_rgb8(128, 128);
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let bytes = bytes.into_inner();
        let id = artwork_id("cover", &bytes);
        let decoded = decoded_image(id, &bytes).unwrap();
        for mode in [
            ArtworkMode::Braille,
            ArtworkMode::Auto,
            ArtworkMode::Braille,
        ] {
            set_grid_artwork_mode(mode);
            let cached = decoded_image(id, &bytes).unwrap();
            assert!(Arc::ptr_eq(&decoded, &cached));
        }
        clear_grid_cache();
        assert!(!Arc::ptr_eq(&decoded, &decoded_image(id, &bytes).unwrap()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "desktop")]
    #[test]
    fn graphical_character_art_uses_the_braille_encoder_and_preserves_aspect_ratio() {
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(16, 16, |x, y| {
            image::Rgb([(x * 16) as u8, (y * 16) as u8, 128])
        }));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let braille =
            String::from_utf8(character_svg(bytes.get_ref(), ArtworkMode::Braille).unwrap())
                .unwrap();
        assert!(
            braille
                .chars()
                .any(|symbol| ('\u{2801}'..='\u{28ff}').contains(&symbol))
        );
        let blocks =
            String::from_utf8(character_svg(bytes.get_ref(), ArtworkMode::Halfblocks).unwrap())
                .unwrap();
        assert!(blocks.contains("width=\"480\" height=\"480\""));
        assert!(blocks.contains("height=\"10\""));
    }
    #[test]
    fn changing_pixels_at_the_same_artwork_path_replaces_the_cached_image() {
        use image::GenericImageView;
        let mut pixels = image::RgbaImage::from_pixel(32, 32, image::Rgba([0, 0, 0, 255]));
        let encode = |pixels| {
            let mut output = std::io::Cursor::new(Vec::new());
            DynamicImage::ImageRgba8(pixels)
                .write_to(&mut output, image::ImageFormat::Bmp)
                .unwrap();
            output.into_inner()
        };
        let first = encode(pixels.clone());
        pixels.put_pixel(16, 16, image::Rgba([255, 255, 255, 255]));
        let second = encode(pixels);
        assert_eq!(&first[..64], &second[..64]);
        assert_eq!(&first[first.len() - 64..], &second[second.len() - 64..]);
        let mut renderer = ArtworkRenderer::new();
        renderer.set_mode(ArtworkMode::Braille);
        assert!(renderer.load_image(&first, "same-path"));
        assert!(renderer.load_image(&second, "same-path"));
        assert_eq!(
            renderer.braille_image.as_ref().unwrap().get_pixel(16, 16).0,
            [255, 255, 255, 255]
        );
    }
}
