//! Album artwork support using ratatui-image.
//!
//! Displays album art in terminals that support graphics protocols
//! (Kitty, iTerm2, Sixel) or falls back to halfblocks.

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
    pub fn new() -> Self {
        // Try to create a picker for the current terminal
        // This queries the terminal for its capabilities
        let picker = Picker::from_query_stdio().ok();

        Self {
            picker,
            protocol: None,
            current_thumb: None,
        }
    }

    /// Check if graphics are supported.
    pub fn is_supported(&self) -> bool {
        self.picker.is_some()
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
            let image = StatefulImage::new(None);
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
