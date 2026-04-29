//! Album artwork support using ratatui-image.
//!
//! Displays album art in terminals that support graphics protocols
//! (Kitty, iTerm2, Sixel) or falls back to halfblocks.

use std::cell::RefCell;
use std::collections::HashMap;

use image::DynamicImage;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use ratatui_image::{picker::Picker, protocol::StatefulProtocol, Resize, StatefulImage};

use crate::app::state::ArtworkMode;

/// Artwork renderer using ratatui-image.
///
/// This struct manages the picker (terminal capability detection) and the
/// current image protocol for rendering.
pub struct ArtworkRenderer {
    picker: Option<Picker>,
    protocol: Option<StatefulProtocol>,
    current_thumb: Option<String>,
    braille_image: Option<DynamicImage>,
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
            braille_image: None,
            mode: ArtworkMode::Auto,
            native_protocol: Some(native),
        }
    }

    /// Check if graphics are supported.
    pub fn is_supported(&self) -> bool {
        self.picker.is_some()
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
        // Skip if same image already loaded
        if self.current_thumb.as_deref() == Some(thumb_path) {
            if self.mode == ArtworkMode::Braille && self.braille_image.is_some() {
                return true;
            }
            if self.mode != ArtworkMode::Braille && self.protocol.is_some() {
                return true;
            }
        }

        // Load image from bytes
        let Ok(img) = image::load_from_memory(image_data) else {
            return false;
        };

        if self.mode == ArtworkMode::Braille {
            self.braille_image = Some(img);
            self.protocol = None;
            self.current_thumb = Some(thumb_path.to_string());
            return true;
        }

        let Some(ref mut picker) = self.picker else {
            return false;
        };

        // Create protocol for rendering (this handles resizing automatically)
        self.protocol = Some(picker.new_resize_protocol(img));
        self.braille_image = None;
        self.current_thumb = Some(thumb_path.to_string());
        true
    }

    /// Load an image with the top portion cropped (for scrolling).
    /// `crop_fraction` is 0.0..1.0 indicating how much of the top to remove.
    pub fn load_image_cropped(&mut self, image_data: &[u8], thumb_path: &str, crop_fraction: f32) -> bool {
        if crop_fraction <= 0.0 {
            return self.load_image(image_data, thumb_path);
        }

        // Cache key includes crop amount to avoid re-creating protocol unnecessarily
        let crop_key = format!("{}:c{}", thumb_path, (crop_fraction * 100.0) as u32);
        if self.current_thumb.as_deref() == Some(&crop_key) {
            return self.has_image();
        }

        let Ok(img) = image::load_from_memory(image_data) else { return false; };

        let crop_pixels = (img.height() as f32 * crop_fraction).min(img.height() as f32 - 1.0) as u32;
        let cropped = if crop_pixels > 0 && crop_pixels < img.height() {
            img.crop_imm(0, crop_pixels, img.width(), img.height() - crop_pixels)
        } else {
            img
        };

        if self.mode == ArtworkMode::Braille {
            self.braille_image = Some(cropped);
            self.protocol = None;
            self.current_thumb = Some(crop_key);
            return true;
        }

        let Some(ref mut picker) = self.picker else { return false; };
        self.protocol = Some(picker.new_resize_protocol(cropped));
        self.braille_image = None;
        self.current_thumb = Some(crop_key);
        true
    }

    /// Clear the current image.
    pub fn clear(&mut self) {
        self.protocol = None;
        self.braille_image = None;
        self.current_thumb = None;
    }

    /// Render the artwork to a frame area.
    ///
    /// Uses `Resize::Crop` so a square cover image fills the entire
    /// box even when the box's cell dimensions don't perfectly
    /// match the terminal's actual cell aspect ratio. The cropped
    /// portion is centered (default Crop behaviour) so a square
    /// album cover loses at most 1-2 pixel rows / cols off the
    /// edges — invisible on most album art.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        if self.mode == ArtworkMode::Braille {
            if let Some(ref img) = self.braille_image {
                render_braille_image(frame, img, area);
            }
            return;
        }
        if let Some(ref mut protocol) = self.protocol {
            let image = StatefulImage::new().resize(Resize::Crop(None));
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

    /// Change the graphics protocol and clear cached images.
    pub fn set_protocol_type(&mut self, protocol_type: ratatui_image::picker::ProtocolType) {
        if let Some(ref mut picker) = self.picker {
            picker.set_protocol_type(protocol_type);
            // Clear cached image so it's re-created with the new protocol
            self.protocol = None;
            self.current_thumb = None;
        }
    }

    /// Restore the native protocol detected at startup and clear cached images.
    pub fn restore_native_protocol(&mut self) {
        if let (Some(ref mut picker), Some(native)) = (&mut self.picker, self.native_protocol) {
            picker.set_protocol_type(native);
            self.protocol = None;
            self.current_thumb = None;
        }
    }

    /// Set the artwork rendering mode and clear all caches.
    pub fn set_mode(&mut self, mode: ArtworkMode) {
        self.mode = mode;
        self.protocol = None;
        self.braille_image = None;
        self.current_thumb = None;
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
fn render_braille_image(frame: &mut Frame, img: &DynamicImage, area: Rect) {
    use image::GenericImageView;

    if area.width == 0 || area.height == 0 {
        return;
    }

    // Braille resolution: 2 dots wide × 4 dots tall per terminal cell
    let pixel_w = area.width as u32 * 2;
    let pixel_h = area.height as u32 * 4;

    let resized = img.resize_exact(pixel_w, pixel_h, image::imageops::FilterType::Triangle);

    // Build luminance histogram for adaptive thresholding
    let mut histogram = [0u32; 256];
    let total_pixels = pixel_w * pixel_h;
    for y in 0..pixel_h {
        for x in 0..pixel_w {
            let pixel = resized.get_pixel(x, y);
            let lum = (pixel[0] as u32 * 299 + pixel[1] as u32 * 587 + pixel[2] as u32 * 114) / 1000;
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

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(area.height as usize);

    for row in 0..area.height as u32 {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(area.width as usize);

        for col in 0..area.width as u32 {
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
            let fg_color = if bright_count > 0 {
                Color::Rgb(
                    (fg_r / bright_count) as u8,
                    (fg_g / bright_count) as u8,
                    (fg_b / bright_count) as u8,
                )
            } else {
                Color::Rgb(0, 0, 0)
            };
            let bg_color = if dark_count > 0 {
                Color::Rgb(
                    (bg_r / dark_count) as u8,
                    (bg_g / dark_count) as u8,
                    (bg_b / dark_count) as u8,
                )
            } else {
                fg_color
            };

            spans.push(Span::styled(
                String::from(ch),
                Style::default().fg(fg_color).bg(bg_color),
            ));
        }

        lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

// ============================================================================
// Album Art Grid — shared Picker and per-album protocol cache
// ============================================================================

thread_local! {
    static GRID_PICKER: RefCell<Option<Picker>> = RefCell::new(None);
    static GRID_PROTOCOLS: RefCell<HashMap<String, StatefulProtocol>> = RefCell::new(HashMap::new());
    static GRID_BRAILLE_IMAGES: RefCell<HashMap<String, DynamicImage>> = RefCell::new(HashMap::new());
    static GRID_ARTWORK_MODE: RefCell<ArtworkMode> = RefCell::new(ArtworkMode::Auto);
    static GRID_NATIVE_PROTOCOL: RefCell<Option<ratatui_image::picker::ProtocolType>> = RefCell::new(None);
}

/// Initialize the grid renderer with a Picker clone.
/// Call once at startup alongside the main artwork renderer init.
pub fn init_grid_renderer(picker: Picker) {
    GRID_NATIVE_PROTOCOL.with(|p| *p.borrow_mut() = Some(picker.protocol_type()));
    GRID_PICKER.with(|p| *p.borrow_mut() = Some(picker));
}

/// Render an album cover image to the given area.
/// Uses a thread-local protocol cache keyed by album rating_key.
/// Returns true if an image was rendered.
pub fn render_grid_image(frame: &mut Frame, area: Rect, key: &str, data: &[u8]) -> bool {
    let mode = GRID_ARTWORK_MODE.with(|m| *m.borrow());

    if mode == ArtworkMode::Braille {
        return render_grid_braille(frame, area, key, data);
    }

    // Protocol-based rendering (Auto/Halfblocks)
    let has_protocol = GRID_PROTOCOLS.with(|protos| protos.borrow().contains_key(key));

    if !has_protocol {
        let created = GRID_PICKER.with(|picker_cell| {
            let mut picker_ref = picker_cell.borrow_mut();
            let Some(picker) = picker_ref.as_mut() else { return false; };
            let Ok(img) = image::load_from_memory(data) else { return false; };
            let protocol = picker.new_resize_protocol(img);
            GRID_PROTOCOLS.with(|protos| {
                protos.borrow_mut().insert(key.to_string(), protocol);
            });
            true
        });
        if !created {
            return false;
        }
    }

    GRID_PROTOCOLS.with(|protos| {
        let mut map = protos.borrow_mut();
        if let Some(protocol) = map.get_mut(key) {
            // Crop-fill so square album thumbs fill non-square cells
            // (terminal cell aspect varies) without leaving gaps.
            let image = StatefulImage::new().resize(Resize::Crop(None));
            frame.render_stateful_widget(image, area, protocol);
            true
        } else {
            false
        }
    })
}

/// Render a grid album cover using braille characters.
fn render_grid_braille(frame: &mut Frame, area: Rect, key: &str, data: &[u8]) -> bool {
    let has_image = GRID_BRAILLE_IMAGES.with(|imgs| imgs.borrow().contains_key(key));

    if !has_image {
        let Ok(img) = image::load_from_memory(data) else { return false; };
        GRID_BRAILLE_IMAGES.with(|imgs| {
            imgs.borrow_mut().insert(key.to_string(), img);
        });
    }

    GRID_BRAILLE_IMAGES.with(|imgs| {
        let map = imgs.borrow();
        if let Some(img) = map.get(key) {
            render_braille_image(frame, img, area);
            true
        } else {
            false
        }
    })
}

/// Clear the grid protocol cache (e.g. on library switch).
pub fn clear_grid_cache() {
    GRID_PROTOCOLS.with(|protos| protos.borrow_mut().clear());
    GRID_BRAILLE_IMAGES.with(|imgs| imgs.borrow_mut().clear());
}

/// Change the grid renderer's graphics protocol and clear cached images.
pub fn set_grid_protocol_type(protocol_type: ratatui_image::picker::ProtocolType) {
    GRID_PICKER.with(|p| {
        if let Some(ref mut picker) = *p.borrow_mut() {
            picker.set_protocol_type(protocol_type);
        }
    });
    clear_grid_cache();
}

/// Restore the grid renderer's native protocol detected at startup.
pub fn restore_grid_native_protocol() {
    let native = GRID_NATIVE_PROTOCOL.with(|p| *p.borrow());
    if let Some(protocol_type) = native {
        set_grid_protocol_type(protocol_type);
    }
}

/// Set the grid renderer's artwork mode and clear caches.
pub fn set_grid_artwork_mode(mode: ArtworkMode) {
    GRID_ARTWORK_MODE.with(|m| *m.borrow_mut() = mode);
    clear_grid_cache();
}
