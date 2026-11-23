//! Pure album-art collage composition.
use image::{ImageFormat, RgbaImage, imageops::FilterType};
use std::io::Cursor;
pub fn compose(covers: Vec<Vec<u8>>) -> anyhow::Result<Vec<u8>> {
    let images: Vec<_> = covers
        .into_iter()
        .filter_map(|bytes| image::load_from_memory(&bytes).ok())
        .take(9)
        .collect();
    anyhow::ensure!(!images.is_empty(), "No album artwork available");
    let count = images.len() as u32;
    let columns = (count as f32).sqrt().ceil() as u32;
    let rows = count.div_ceil(columns);
    let mut canvas = RgbaImage::new(512, 512);
    for (index, image) in images.into_iter().enumerate() {
        let index = index as u32;
        let row = index / columns;
        let column = index % columns;
        let row_count = columns.min(count - row * columns);
        let x = 512 * column / row_count;
        let right = 512 * (column + 1) / row_count;
        let y = 512 * row / rows;
        let bottom = 512 * (row + 1) / rows;
        let tile = image
            .resize_to_fill(right - x, bottom - y, FilterType::Lanczos3)
            .into_rgba8();
        image::imageops::overlay(&mut canvas, &tile, i64::from(x), i64::from(y));
    }
    let mut output = Cursor::new(Vec::new());
    canvas.write_to(&mut output, ImageFormat::Png)?;
    Ok(output.into_inner())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mosaic_uses_up_to_nine_covers_without_repeating_missing_tiles() {
        let covers = (0..12)
            .map(|index| {
                let image = RgbaImage::from_pixel(3, 3, image::Rgba([index * 20, 0, 0, 255]));
                let mut data = Cursor::new(Vec::new());
                image.write_to(&mut data, ImageFormat::Png).unwrap();
                data.into_inner()
            })
            .collect();
        let image = image::load_from_memory(&compose(covers).unwrap())
            .unwrap()
            .into_rgba8();
        assert_eq!(image.dimensions(), (512, 512));
        assert_eq!(image[(256, 256)][0], 80);
        assert_eq!(image[(500, 500)][0], 160);
        assert!(compose(vec![vec![]]).is_err());
    }
}
