//! Album artwork support using ratatui-image.
//!
//! Displays album art in terminals that support graphics protocols
//! (Kitty, iTerm2, Sixel) or falls back to halfblocks.

use std::cell::RefCell;
use std::collections::HashMap;

use ratatui::prelude::*;
use ratatui_image::{picker::Picker, protocol::StatefulProtocol, StatefulImage};

/// Artwork renderer using ratatui-image.
///
/// This struct manages the picker (terminal capability detection) and the
/// current image protocol for rendering.
pub struct ArtworkRenderer {
    picker: Option<Picker>,
    protocol: Option<StatefulProtocol>,
    current_thumb: Option<String>,
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
        }
    }

    /// Create with a pre-initialized picker for graphics support.
    pub fn new_with_picker(picker: Picker) -> Self {
        tracing::info!("Artwork protocol: {:?}", picker.protocol_type());
        Self {
            picker: Some(picker),
            protocol: None,
            current_thumb: None,
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
                ratatui_image::picker::ProtocolType::Halfblocks => "Halfblocks",
                ratatui_image::picker::ProtocolType::Sixel => "Sixel",
                ratatui_image::picker::ProtocolType::Kitty => "Kitty",
                ratatui_image::picker::ProtocolType::Iterm2 => "iTerm2",
            },
            None => "None",
        }
    }

    /// Load and prepare an image for rendering.
    /// Returns true if the image was loaded successfully.
    pub fn load_image(&mut self, image_data: &[u8], thumb_path: &str) -> bool {
        // Skip if same image already loaded
        if self.current_thumb.as_deref() == Some(thumb_path) && self.protocol.is_some() {
            return true;
        }

        let Some(ref mut picker) = self.picker else {
            return false;
        };

        // Load image from bytes
        let Ok(img) = image::load_from_memory(image_data) else {
            return false;
        };

        // Create protocol for rendering (this handles resizing automatically)
        self.protocol = Some(picker.new_resize_protocol(img));
        self.current_thumb = Some(thumb_path.to_string());
        true
    }

    /// Clear the current image.
    pub fn clear(&mut self) {
        self.protocol = None;
        self.current_thumb = None;
    }

    /// Render the artwork to a frame area.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        if let Some(ref mut protocol) = self.protocol {
            let image = StatefulImage::new();
            frame.render_stateful_widget(image, area, protocol);
        }
    }

    /// Check if there's an image loaded.
    pub fn has_image(&self) -> bool {
        self.protocol.is_some()
    }

    /// Get the current thumb path.
    pub fn current_thumb(&self) -> Option<&str> {
        self.current_thumb.as_deref()
    }
}

// ============================================================================
// Album Art Grid — shared Picker and per-album protocol cache
// ============================================================================

thread_local! {
    static GRID_PICKER: RefCell<Option<Picker>> = RefCell::new(None);
    static GRID_PROTOCOLS: RefCell<HashMap<String, StatefulProtocol>> = RefCell::new(HashMap::new());
}

/// Initialize the grid renderer with a Picker clone.
/// Call once at startup alongside the main artwork renderer init.
pub fn init_grid_renderer(picker: Picker) {
    GRID_PICKER.with(|p| *p.borrow_mut() = Some(picker));
}

/// Render an album cover image to the given area.
/// Uses a thread-local protocol cache keyed by album rating_key.
/// Returns true if an image was rendered.
pub fn render_grid_image(frame: &mut Frame, area: Rect, key: &str, data: &[u8]) -> bool {
    // Ensure protocol is cached
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

    // Render from cache
    GRID_PROTOCOLS.with(|protos| {
        let mut map = protos.borrow_mut();
        if let Some(protocol) = map.get_mut(key) {
            let image = StatefulImage::new();
            frame.render_stateful_widget(image, area, protocol);
            true
        } else {
            false
        }
    })
}

/// Clear the grid protocol cache (e.g. on library switch).
pub fn clear_grid_cache() {
    GRID_PROTOCOLS.with(|protos| protos.borrow_mut().clear());
}
