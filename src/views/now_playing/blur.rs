// SPDX-License-Identifier: GPL-3.0

//! Blurred cover art generation and caching for the playback bar background.

use image::{DynamicImage, ImageFormat};
use std::io::Cursor;

/// Compute a blurred version of the cover art for use as a background.
///
/// The image is resized to 400px wide (maintaining aspect ratio) and then
/// blurred with a Gaussian filter (sigma = 30.0). The result is encoded
/// as PNG bytes.
///
/// Returns `None` if the input bytes cannot be decoded as an image.
pub fn compute_blurred_cover(bytes: &[u8]) -> Option<Vec<u8>> {
    // Load image from memory
    let img = image::load_from_memory(bytes).ok()?;

    // Resize to 400px wide, maintaining aspect ratio
    let resized = resize_to_width(&img, 400);

    // Apply Gaussian blur with sigma = 30.0
    let blurred = image::imageops::blur(&resized, 30.0);

    // Encode to PNG
    let mut output = Vec::new();
    let mut cursor = Cursor::new(&mut output);
    blurred.write_to(&mut cursor, ImageFormat::Png).ok()?;

    Some(output)
}

/// Resize an image to a specific width, maintaining aspect ratio.
fn resize_to_width(img: &DynamicImage, target_width: u32) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w == 0 {
        return img.clone();
    }
    let ratio = target_width as f32 / w as f32;
    let target_height = (h as f32 * ratio) as u32;

    img.resize(
        target_width,
        target_height,
        image::imageops::FilterType::Triangle,
    )
}
