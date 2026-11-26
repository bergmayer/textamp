//! Owned libghostty-vt render state. No handle crosses its renderer thread.
use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::{Arc, OnceLock},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Terminal renderer failed during {operation} (code {code})")]
    Library { operation: &'static str, code: i32 },
    #[error("Terminal renderer returned invalid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

/// One complete grapheme cluster and its resolved render attributes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    pub flags: u16,
    pub wide: u8,
    pub selected: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub visible: u8,
    pub blinking: u8,
    pub wide_tail: u8,
    pub style: u8,
}

/// Complete snapshot with structurally shared unchanged rows; watch receivers may coalesce it.
#[derive(Clone, Debug)]
pub struct Frame {
    pub columns: u16,
    pub rows: u16,
    pub cells: Vec<Arc<Vec<Cell>>>,
    pub dirty_rows: Vec<u16>,
    pub cursor: Cursor,
    pub images: Vec<Image>,
}

/// Decoded texture shared across unchanged placements and frame snapshots.
#[derive(Clone, Debug)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
/// Image placement measured by Ghostty in physical pixels and terminal cells.
#[derive(Clone, Debug)]
pub struct Image {
    pub id: u32,
    pub placement: u32,
    pub generation: u64,
    pub column: i32,
    pub row: i32,
    pub z: i32,
    pub layer: u8,
    pub offset: [u32; 2],
    pub size: [u32; 2],
    pub source: [u32; 4],
    pub pixels: Arc<Pixels>,
}
impl Image {
    /// Equality of a placement without comparing immutable texture bytes again.
    pub fn same_placement(&self, other: &Self) -> bool {
        self.id == other.id
            && self.placement == other.placement
            && self.generation == other.generation
            && self.column == other.column
            && self.row == other.row
            && self.z == other.z
            && self.layer == other.layer
            && self.offset == other.offset
            && self.size == other.size
            && self.source == other.source
    }
}
#[repr(C)]
#[derive(Default)]
struct ImageMetadata {
    id: u32,
    placement: u32,
    generation: u64,
    column: i32,
    row: i32,
    z: i32,
    offset_x: u32,
    offset_y: u32,
    draw_width: u32,
    draw_height: u32,
    source_x: u32,
    source_y: u32,
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    layer: u8,
    format: u8,
    pixels: *const u8,
    length: usize,
}
#[repr(C)]
#[derive(Default)]
struct CellMetadata {
    foreground: [u8; 3],
    background: [u8; 3],
    flags: u16,
    wide: u8,
    selected: u8,
}
#[repr(C)]
struct PngImage {
    width: u32,
    height: u32,
    data: *mut u8,
    data_len: usize,
}
type PngDecoder =
    unsafe extern "C" fn(*mut c_void, *const c_void, *const u8, usize, *mut PngImage) -> bool;
unsafe extern "C" {
    fn textamp_vt_new(columns: u16, rows: u16, error: *mut i32) -> *mut c_void;
    fn textamp_vt_free(vt: *mut c_void);
    fn textamp_vt_write(vt: *mut c_void, bytes: *const u8, length: usize);
    fn textamp_vt_resize(vt: *mut c_void, columns: u16, rows: u16, cw: u32, ch: u32) -> i32;
    fn textamp_vt_update(vt: *mut c_void, dirty: *mut u8, cursor: *mut Cursor) -> i32;
    fn textamp_vt_row(vt: *mut c_void, y: *mut i32, dirty: *mut u8) -> i32;
    fn textamp_vt_cell(
        vt: *mut c_void,
        x: u16,
        metadata: *mut CellMetadata,
        text: *mut u8,
        capacity: usize,
        length: *mut usize,
    ) -> i32;
    fn textamp_vt_finish(vt: *mut c_void);
    fn textamp_vt_images_start(vt: *mut c_void) -> i32;
    fn textamp_vt_image(vt: *mut c_void, out: *mut ImageMetadata) -> i32;
    fn textamp_vt_png_decoder(decoder: PngDecoder) -> i32;
    fn textamp_vt_image_alloc(allocator: *const c_void, length: usize) -> *mut u8;
}
fn check(code: i32, operation: &'static str) -> Result<(), Error> {
    if code == 0 {
        Ok(())
    } else {
        Err(Error::Library { operation, code })
    }
}

// # Safety
// Ghostty supplies valid input and output buffers for the duration of the callback.
// The returned pixel allocation uses Ghostty's supplied allocator; ownership transfers
// to Ghostty only on success, with the exact allocation length recorded in the output.
unsafe extern "C" fn decode_png(
    _: *mut c_void,
    allocator: *const c_void,
    data: *const u8,
    length: usize,
    out: *mut PngImage,
) -> bool {
    // # Safety: the library owns this readable input buffer throughout the callback.
    let bytes = unsafe { std::slice::from_raw_parts(data, length) };
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let Ok(image) = reader.decode() else {
        return false;
    };
    let pixels = image.to_rgba8();
    let (width, height) = pixels.dimensions();
    let bytes = pixels.as_raw();
    // # Safety: allocator is valid for this callback; the result is either null or
    // writable for bytes.len(), and out points to Ghostty's initialized output.
    unsafe {
        let allocation = textamp_vt_image_alloc(allocator, bytes.len());
        if allocation.is_null() {
            return false;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), allocation, bytes.len());
        out.write(PngImage {
            width,
            height,
            data: allocation,
            data_len: bytes.len(),
        });
    }
    true
}

/// A VT parser, render-state snapshot and reusable row/cell iterators.
pub struct Vt {
    handle: NonNull<c_void>,
    columns: u16,
    rows: u16,
    previous: Vec<Arc<Vec<Cell>>>,
    images: std::collections::HashMap<(u32, u64), Arc<Pixels>>,
}
impl Vt {
    /// Create a terminal with host-provided PNG decoding and bounded image storage.
    /// # Errors
    /// Returns allocation, feature or system-callback initialization failures.
    pub fn new(columns: u16, rows: u16) -> Result<Self, Error> {
        static DECODER: OnceLock<i32> = OnceLock::new();
        // # Safety: the callback has static lifetime and honors Ghostty's allocator contract.
        let code = *DECODER.get_or_init(|| unsafe { textamp_vt_png_decoder(decode_png) });
        check(code, "PNG decoder initialization")?;
        let mut error = 0;
        // # Safety: dimensions are positive and error is a writable result slot.
        let handle = unsafe { textamp_vt_new(columns.max(1), rows.max(1), &mut error) };
        let handle = NonNull::new(handle).ok_or(Error::Library {
            operation: "creation",
            code: error,
        })?;
        Ok(Self {
            handle,
            columns: columns.max(1),
            rows: rows.max(1),
            previous: Vec::new(),
            images: Default::default(),
        })
    }
    /// Feed the actual Ratatui VT output, including terminal graphics sequences.
    pub fn write(&mut self, bytes: &[u8]) {
        // # Safety: the handle is exclusively owned and bytes remains valid until return.
        unsafe { textamp_vt_write(self.handle.as_ptr(), bytes.as_ptr(), bytes.len()) };
    }
    /// Resize the cell grid and the physical cell dimensions used by image placements.
    /// # Errors
    /// Propagates allocation or invalid-size errors from the VT library.
    pub fn resize(&mut self, columns: u16, rows: u16, cw: u32, ch: u32) -> Result<(), Error> {
        // # Safety: the live handle is exclusive; all dimensions are positive.
        check(
            unsafe {
                textamp_vt_resize(
                    self.handle.as_ptr(),
                    columns.max(1),
                    rows.max(1),
                    cw.max(1),
                    ch.max(1),
                )
            },
            "resize",
        )?;
        self.columns = columns.max(1);
        self.rows = rows.max(1);
        self.previous.clear();
        Ok(())
    }
    /// Read dirty rows and full grapheme clusters from Ghostty's render-state API.
    /// # Errors
    /// Propagates render-state and invalid-text errors; no plain-text screen extraction is used.
    pub fn frame(&mut self) -> Result<Frame, Error> {
        let mut dirty = 0;
        let mut cursor = Cursor::default();
        // # Safety: all out parameters are initialized writable slots and the live handle is exclusive.
        check(
            unsafe { textamp_vt_update(self.handle.as_ptr(), &mut dirty, &mut cursor) },
            "render update",
        )?;
        let full = self.previous.len() != usize::from(self.rows);
        if full {
            let empty = Arc::new(vec![Cell::default(); usize::from(self.columns)]);
            self.previous = vec![empty; usize::from(self.rows)];
        }
        let mut dirty_rows = Vec::new();
        let mut y = 0;
        let mut changed = 0;
        // # Safety: iterators are owned by the handle and are consumed before the next update.
        while unsafe { textamp_vt_row(self.handle.as_ptr(), &mut y, &mut changed) } != 0 {
            let Ok(index) = usize::try_from(y) else {
                continue;
            };
            if index >= self.previous.len() || (!full && changed == 0 && dirty != 2) {
                continue;
            }
            let mut cells = Vec::with_capacity(usize::from(self.columns));
            for x in 0..self.columns {
                let mut metadata = CellMetadata::default();
                let mut bytes = vec![0; 32];
                let mut length = 0;
                // # Safety: metadata, length and bytes are writable; the row iterator is current.
                let mut result = unsafe {
                    textamp_vt_cell(
                        self.handle.as_ptr(),
                        x,
                        &mut metadata,
                        bytes.as_mut_ptr(),
                        bytes.len(),
                        &mut length,
                    )
                };
                if length > bytes.len() {
                    bytes.resize(length, 0);
                    // # Safety: the same cell is still current and the resized buffer has the reported capacity.
                    result = unsafe {
                        textamp_vt_cell(
                            self.handle.as_ptr(),
                            x,
                            &mut metadata,
                            bytes.as_mut_ptr(),
                            bytes.len(),
                            &mut length,
                        )
                    };
                }
                check(result, "grapheme read")?;
                bytes.truncate(length);
                cells.push(Cell {
                    text: String::from_utf8(bytes)?,
                    foreground: metadata.foreground,
                    background: metadata.background,
                    flags: metadata.flags,
                    wide: metadata.wide,
                    selected: metadata.selected != 0,
                });
            }
            self.previous[index] = Arc::new(cells);
            dirty_rows.push(index as u16);
        }
        let images = self.image_placements()?;
        // # Safety: all iterators are consumed; acknowledge this snapshot before the next update.
        unsafe { textamp_vt_finish(self.handle.as_ptr()) };
        Ok(Frame {
            columns: self.columns,
            rows: self.rows,
            cells: self.previous.clone(),
            dirty_rows,
            cursor,
            images,
        })
    }
    fn image_placements(&mut self) -> Result<Vec<Image>, Error> {
        // # Safety: the render state is current and the image iterator is exclusively owned.
        check(
            unsafe { textamp_vt_images_start(self.handle.as_ptr()) },
            "image iterator",
        )?;
        let mut result = Vec::new();
        let mut used = std::collections::HashSet::new();
        loop {
            let mut image = ImageMetadata::default();
            // # Safety: image is a writable output and no terminal update occurs during iteration.
            let status = unsafe { textamp_vt_image(self.handle.as_ptr(), &mut image) };
            if status == 0 {
                break;
            }
            if status < 0 {
                continue;
            }
            let key = (image.id, image.generation);
            used.insert(key);
            let pixels = if let Some(pixels) = self.images.get(&key) {
                pixels.clone()
            } else {
                // # Safety: Ghostty owns these pixels until the next terminal modification;
                // the C shim returns the exact length from its image storage.
                let bytes = unsafe { std::slice::from_raw_parts(image.pixels, image.length) };
                let rgba = if image.format == 0 {
                    bytes
                        .chunks_exact(3)
                        .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
                        .collect()
                } else {
                    bytes.to_vec()
                };
                let pixels = Arc::new(Pixels {
                    width: image.width,
                    height: image.height,
                    rgba,
                });
                self.images.insert(key, pixels.clone());
                pixels
            };
            result.push(Image {
                id: image.id,
                placement: image.placement,
                generation: image.generation,
                column: image.column,
                row: image.row,
                z: image.z,
                layer: image.layer,
                offset: [image.offset_x, image.offset_y],
                size: [image.draw_width, image.draw_height],
                source: [
                    image.source_x,
                    image.source_y,
                    image.source_width,
                    image.source_height,
                ],
                pixels,
            });
        }
        self.images.retain(|key, _| used.contains(key));
        result.sort_by_key(|image| (image.layer, image.z));
        Ok(result)
    }
}
impl Drop for Vt {
    fn drop(&mut self) {
        // # Safety: this is the unique owning handle and all borrowed render data has been copied.
        unsafe { textamp_vt_free(self.handle.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kitty_png_pixels_and_placements_use_the_host_decoder() {
        use base64::Engine;
        let pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba([12, 34, 56, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(bytes.into_inner());
        let mut vt = Vt::new(20, 6).unwrap();
        vt.resize(20, 6, 9, 18).unwrap();
        vt.write(format!("\x1b_Ga=T,f=100,s=2,v=2,i=7,c=2,r=2,q=2;{data}\x1b\\").as_bytes());
        let frame = vt.frame().unwrap();
        assert_eq!(frame.images.len(), 1);
        assert_eq!(frame.images[0].id, 7);
        assert_eq!(&frame.images[0].pixels.rgba[..4], &[12, 34, 56, 255]);
        assert!(frame.images[0].size[0] > 0);
    }
    #[test]
    fn skin_tone_emoji_occupies_two_cells_with_grapheme_clustering() {
        let mut vt = Vt::new(10, 2).unwrap();
        vt.write("\x1b[?2027h☝🏻x".as_bytes());
        let frame = vt.frame().unwrap();
        assert_eq!(frame.cells[0][0].text, "☝🏻");
        assert_eq!(frame.cells[0][0].wide, 1);
        assert_eq!(frame.cells[0][1].wide, 2);
        assert_eq!(frame.cells[0][2].text, "x");
    }

    #[test]
    fn render_state_preserves_graphemes_wide_cells_colors_and_dirty_rows() {
        let mut vt = Vt::new(20, 4).unwrap();
        vt.write("\x1b[38;2;12;34;56me\u{301}界\x1b[0m\r\n┌──┐".as_bytes());
        let frame = vt.frame().unwrap();
        assert_eq!(frame.cells[0][0].text, "e\u{301}");
        assert_eq!(frame.cells[0][0].foreground, [12, 34, 56]);
        assert_eq!(frame.cells[0][1].text, "界");
        assert_eq!(frame.cells[0][1].wide, 1);
        assert_eq!(frame.cells[0][2].wide, 2);
        assert_eq!(frame.cells[1][0].text, "┌");
        assert!(vt.frame().unwrap().dirty_rows.is_empty());
        vt.write(b"\x1b[2;5Hupdated");
        let updated = vt.frame().unwrap();
        assert_eq!(updated.cells[0][0].text, "e\u{301}");
        assert_eq!(updated.cells[1][4].text, "u");

        vt.resize(30, 6, 9, 18).unwrap();
        assert_eq!(vt.frame().unwrap().columns, 30);
    }
}
