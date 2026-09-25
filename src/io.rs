//! Load photos and write PNG or WebP files.

use anyhow::{Context, Result, bail};
use image::imageops::FilterType;
use image::{ImageDecoder, ImageReader, RgbaImage};
use std::path::Path;

/// Longest side kept after loading. Character art samples the photo in cells,
/// so a few thousand pixels on a side is enough detail for the densest grid.
const SOURCE_EDGE: u32 = 4096;

pub fn load_image(path: &Path) -> Result<RgbaImage> {
    let reader = ImageReader::open(path)
        .with_context(|| format!("could not open {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("could not recognize {}", path.display()))?;
    let mut decoder = reader
        .into_decoder()
        .with_context(|| format!("could not decode {}", path.display()))?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = image::DynamicImage::from_decoder(decoder)
        .with_context(|| format!("could not decode {}", path.display()))?;
    image.apply_orientation(orientation);
    let rgba = fit_edge(&image.into_rgba8(), SOURCE_EDGE);
    if rgba.width() == 0 || rgba.height() == 0 {
        bail!("{} is an empty image", path.display());
    }
    Ok(rgba)
}

pub fn save_image(path: &Path, image: &RgbaImage) -> Result<()> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" | "webp" => image
            .save(path)
            .with_context(|| format!("could not write {}", path.display())),
        _ => bail!("save the picture as a .png or .webp file"),
    }
}

/// Shrink `image` so its longest side is at most `max_edge`.
pub fn fit_edge(image: &RgbaImage, max_edge: u32) -> RgbaImage {
    let width = image.width();
    let height = image.height();
    let edge = width.max(height);
    if edge <= max_edge || edge == 0 {
        return image.clone();
    }
    let scale = max_edge as f32 / edge as f32;
    let target_w = ((width as f32) * scale).round().max(1.0) as u32;
    let target_h = ((height as f32) * scale).round().max(1.0) as u32;
    image::imageops::resize(image, target_w, target_h, FilterType::Triangle)
}
