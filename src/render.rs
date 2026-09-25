//! Sample a photo into cells and stamp a Japanese character into each one.

use ab_glyph::{Font, FontVec};
use anyhow::{Result, bail};
use image::RgbaImage;
use std::time::Instant;

use crate::fontfind;
use crate::glyphs::{self, Atlas, FontLayout};
use crate::params::{ColorMode, Params, Rgb, Style};

pub use crate::glyphs::FontMetrics;

#[derive(Clone, Copy, Debug)]
pub struct GridSpec {
    pub columns: u32,
    pub rows: u32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub width: u32,
    pub height: u32,
    pub cell_px: u32,
    pub capped: bool,
}

#[derive(Clone, Debug)]
pub struct Stats {
    pub columns: u32,
    pub rows: u32,
    pub width: u32,
    pub height: u32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub levels: usize,
    pub capped: bool,
    pub elapsed_ms: u128,
    pub font: String,
}

pub struct Rendered {
    pub image: RgbaImage,
    pub ramp: RgbaImage,
    pub stats: Stats,
    /// Ramp characters from light to dark.
    pub characters: String,
}

/// Longest side of a saved picture. Larger requests shrink the character size.
const MAX_EDGE: u32 = 8192;

pub struct Engine {
    font: FontVec,
    layout: FontLayout,
    family: String,
    atlas: Option<CachedAtlas>,
    sample_key: Option<SampleKey>,
    samples: Vec<Sample>,
}

struct CachedAtlas {
    style: Style,
    cell_px: u32,
    atlas: Atlas,
}

struct SampleKey {
    source_id: u64,
    columns: u32,
    rows: u32,
}

#[derive(Clone, Copy)]
struct Sample {
    r: f32,
    g: f32,
    b: f32,
    luma: f32,
}

impl Engine {
    pub fn open() -> Result<Self> {
        let source = fontfind::locate()?;
        let font = fontfind::load(&source)?;
        if font.glyph_id('あ').0 == 0 {
            bail!(
                "{} does not include hiragana. Pick a Japanese face with SUMI_FONT.",
                source.path.display()
            );
        }
        let layout = glyphs::probe_layout(&font)?;
        Ok(Self {
            font,
            layout,
            family: source.family,
            atlas: None,
            sample_key: None,
            samples: Vec::new(),
        })
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    pub fn metrics(&self) -> FontMetrics {
        self.layout.metrics
    }

    pub fn clear_cache(&mut self) {
        self.atlas = None;
        self.sample_key = None;
        self.samples.clear();
    }

    pub fn render(
        &mut self,
        source_id: u64,
        image: &RgbaImage,
        params: &Params,
    ) -> Result<Rendered> {
        let params = params.sanitize();
        let started = Instant::now();
        if image.width() == 0 || image.height() == 0 {
            bail!("the picture has no pixels");
        }
        let grid = output_size(&self.layout.metrics, image.width(), image.height(), &params);
        self.ensure_atlas(params.style, grid.cell_px)?;
        self.ensure_samples(source_id, image, grid.columns, grid.rows);

        let atlas = self
            .atlas
            .as_ref()
            .expect("atlas was just built")
            .atlas_ref();
        let ramp = glyphs::select_ramp(&atlas.glyphs, params.levels as usize);
        if ramp.is_empty() {
            bail!("no characters were available to draw with");
        }

        let ink = tone_map(&self.samples, grid.columns, grid.rows, &params);
        let ramp_cov: Vec<f32> = ramp
            .iter()
            .map(|&index| atlas.glyphs[index].coverage)
            .collect();
        let chosen = choose_glyphs(&ink, grid.columns, grid.rows, &ramp_cov, params.dither);
        let characters: String = ramp.iter().map(|&index| atlas.glyphs[index].ch).collect();
        let image = stamp(
            &self.samples,
            &chosen,
            &ramp,
            atlas,
            &params,
            grid.columns,
            grid.rows,
        );
        let ramp_image = stamp_ramp(atlas, &ramp, &params);

        Ok(Rendered {
            image,
            ramp: ramp_image,
            characters,
            stats: Stats {
                columns: grid.columns,
                rows: grid.rows,
                width: grid.width,
                height: grid.height,
                cell_w: grid.cell_w,
                cell_h: grid.cell_h,
                levels: ramp.len(),
                capped: grid.capped,
                elapsed_ms: started.elapsed().as_millis(),
                font: self.family.clone(),
            },
        })
    }

    fn ensure_atlas(&mut self, style: Style, cell_px: u32) -> Result<()> {
        let reusable = self
            .atlas
            .as_ref()
            .is_some_and(|cached| cached.style == style && cached.cell_px == cell_px);
        if reusable {
            return Ok(());
        }
        let atlas = glyphs::build_atlas(&self.font, &self.layout, style, cell_px)?;
        self.atlas = Some(CachedAtlas {
            style,
            cell_px,
            atlas,
        });
        Ok(())
    }

    fn ensure_samples(&mut self, source_id: u64, image: &RgbaImage, columns: u32, rows: u32) {
        let same = self.sample_key.as_ref().is_some_and(|key| {
            key.source_id == source_id && key.columns == columns && key.rows == rows
        });
        if same {
            return;
        }
        self.samples = sample_cells(image, columns, rows);
        self.sample_key = Some(SampleKey {
            source_id,
            columns,
            rows,
        });
    }
}

impl CachedAtlas {
    fn atlas_ref(&self) -> &Atlas {
        &self.atlas
    }
}

pub fn output_size(metrics: &FontMetrics, image_w: u32, image_h: u32, params: &Params) -> GridSpec {
    let params = params.sanitize();
    let mut cell_px = params.cell_px;
    let mut columns = params.columns;
    let mut capped = false;

    loop {
        let (cell_w, cell_h) = glyphs::cell_size(params.style, cell_px, metrics);
        let rows = rows_for(image_w, image_h, columns, cell_w, cell_h);
        let width = columns.saturating_mul(cell_w);
        let height = rows.saturating_mul(cell_h);
        if width <= MAX_EDGE && height <= MAX_EDGE {
            return GridSpec {
                columns,
                rows,
                cell_w,
                cell_h,
                width,
                height,
                cell_px,
                capped,
            };
        }
        capped = true;
        if cell_px > 8 {
            cell_px -= 1;
            continue;
        }
        if columns > 8 {
            columns -= 1;
            continue;
        }
        return GridSpec {
            columns,
            rows,
            cell_w,
            cell_h,
            width,
            height,
            cell_px,
            capped,
        };
    }
}

pub fn rows_for(image_w: u32, image_h: u32, columns: u32, cell_w: u32, cell_h: u32) -> u32 {
    let image_w = u64::from(image_w.max(1));
    let image_h = u64::from(image_h.max(1));
    let columns = u64::from(columns.max(1));
    let cell_w = u64::from(cell_w.max(1));
    let cell_h = u64::from(cell_h.max(1));
    (columns * cell_w * image_h / (cell_h * image_w)).max(1) as u32
}

fn sample_cells(image: &RgbaImage, columns: u32, rows: u32) -> Vec<Sample> {
    let width = image.width();
    let height = image.height();
    let raw = image.as_raw();
    let mut out = vec![
        Sample {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            luma: 1.0,
        };
        (columns * rows) as usize
    ];

    for row in 0..rows {
        let y0 = (u64::from(row) * u64::from(height) / u64::from(rows)) as u32;
        let y1 = next_edge(row, rows, height, y0);
        for col in 0..columns {
            let x0 = (u64::from(col) * u64::from(width) / u64::from(columns)) as u32;
            let x1 = next_edge(col, columns, width, x0);
            let mut sr = 0.0f32;
            let mut sg = 0.0f32;
            let mut sb = 0.0f32;
            let mut sa = 0.0f32;
            for y in y0..y1 {
                let row_off = y as usize * width as usize * 4;
                for x in x0..x1 {
                    let offset = row_off + x as usize * 4;
                    let alpha = raw[offset + 3] as f32 * (1.0 / 255.0);
                    sr += raw[offset] as f32 * alpha;
                    sg += raw[offset + 1] as f32 * alpha;
                    sb += raw[offset + 2] as f32 * alpha;
                    sa += alpha;
                }
            }
            if sa > 1.0e-3 {
                let r = (sr / sa / 255.0).clamp(0.0, 1.0);
                let g = (sg / sa / 255.0).clamp(0.0, 1.0);
                let b = (sb / sa / 255.0).clamp(0.0, 1.0);
                out[(row * columns + col) as usize] = Sample {
                    r,
                    g,
                    b,
                    luma: 0.2126 * r + 0.7152 * g + 0.0722 * b,
                };
            }
        }
    }
    out
}

fn next_edge(index: u32, count: u32, limit: u32, start: u32) -> u32 {
    let mut end = ((u64::from(index) + 1) * u64::from(limit) / u64::from(count)) as u32;
    if end <= start {
        end = start.saturating_add(1);
    }
    end.min(limit).max(start)
}

fn tone_map(samples: &[Sample], columns: u32, rows: u32, params: &Params) -> Vec<f32> {
    let mut luma: Vec<f32> = samples
        .iter()
        .map(|sample| {
            if params.invert {
                1.0 - sample.luma
            } else {
                sample.luma
            }
        })
        .collect();

    if params.stretch > 0.0 {
        let (lo, hi) = percentile(&luma, 0.02, 0.98);
        if hi - lo >= 1.0e-3 {
            for value in &mut luma {
                let stretched = ((*value - lo) / (hi - lo)).clamp(0.0, 1.0);
                *value =
                    (*value * (1.0 - params.stretch) + stretched * params.stretch).clamp(0.0, 1.0);
            }
        }
    }

    let edges = if params.outlines > 0.0 {
        edge_map(&luma, columns, rows)
    } else {
        vec![0.0; luma.len()]
    };

    luma.iter()
        .zip(edges)
        .map(|(value, edge)| {
            let mut tone = (*value - 0.5) * params.contrast + 0.5 + params.brightness;
            tone = tone.clamp(0.0, 1.0).powf(params.gamma);
            (1.0 - tone + edge * params.outlines).clamp(0.0, 1.0)
        })
        .collect()
}

fn percentile(values: &[f32], low: f32, high: f32) -> (f32, f32) {
    if values.is_empty() {
        return (0.0, 1.0);
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let last = sorted.len() - 1;
    let lo = sorted[(last as f32 * low).round() as usize];
    let hi = sorted[(last as f32 * high).round() as usize];
    (lo, hi)
}

fn edge_map(luma: &[f32], columns: u32, rows: u32) -> Vec<f32> {
    let mut magnitude = vec![0.0f32; luma.len()];
    if columns < 3 || rows < 3 {
        return magnitude;
    }
    for y in 1..rows - 1 {
        for x in 1..columns - 1 {
            let sample = |dx: i32, dy: i32| {
                luma[((y as i32 + dy) as u32 * columns + (x as i32 + dx) as u32) as usize]
            };
            let gx = -sample(-1, -1) + sample(1, -1) - 2.0 * sample(-1, 0) + 2.0 * sample(1, 0)
                - sample(-1, 1)
                + sample(1, 1);
            let gy = -sample(-1, -1) - 2.0 * sample(0, -1) - sample(1, -1)
                + sample(-1, 1)
                + 2.0 * sample(0, 1)
                + sample(1, 1);
            magnitude[(y * columns + x) as usize] = (gx * gx + gy * gy).sqrt();
        }
    }
    let (_lo, hi) = percentile(&magnitude, 0.5, 0.92);
    if hi < 1.0e-4 {
        return magnitude;
    }
    for value in &mut magnitude {
        *value = (*value / hi).clamp(0.0, 1.0);
    }
    magnitude
}

fn choose_glyphs(ink: &[f32], columns: u32, rows: u32, ramp: &[f32], dither: bool) -> Vec<u16> {
    let mut buffer = ink.to_vec();
    let mut chosen = vec![0u16; ink.len()];
    let lo = ramp[0];
    let span = (ramp[ramp.len() - 1] - lo).max(1.0e-4);

    for y in 0..rows {
        let reverse = dither && y % 2 == 1;
        for step in 0..columns {
            let x = if reverse { columns - 1 - step } else { step };
            let index = (y * columns + x) as usize;
            let value = buffer[index].clamp(0.0, 1.0);
            let target = lo + value * span;
            let glyph = nearest(target, ramp);
            chosen[index] = glyph as u16;
            if dither {
                let error = (target - ramp[glyph]) / span;
                let dir = if reverse { -1 } else { 1 };
                diffuse(&mut buffer, columns, rows, x as i32, y as i32, error, dir);
            }
        }
    }
    chosen
}

fn nearest(target: f32, ramp: &[f32]) -> usize {
    let index = ramp.partition_point(|coverage| *coverage < target);
    if index == 0 {
        0
    } else if index >= ramp.len() {
        ramp.len() - 1
    } else if target - ramp[index - 1] <= ramp[index] - target {
        index - 1
    } else {
        index
    }
}

fn diffuse(buf: &mut [f32], columns: u32, rows: u32, x: i32, y: i32, error: f32, dir: i32) {
    add_error(buf, columns, rows, x + dir, y, error * (7.0 / 16.0));
    add_error(buf, columns, rows, x - dir, y + 1, error * (3.0 / 16.0));
    add_error(buf, columns, rows, x, y + 1, error * (5.0 / 16.0));
    add_error(buf, columns, rows, x + dir, y + 1, error * (1.0 / 16.0));
}

fn add_error(buf: &mut [f32], columns: u32, rows: u32, x: i32, y: i32, delta: f32) {
    if x < 0 || y < 0 {
        return;
    }
    let (x, y) = (x as u32, y as u32);
    if x >= columns || y >= rows {
        return;
    }
    buf[(y * columns + x) as usize] += delta;
}

fn stamp(
    samples: &[Sample],
    chosen: &[u16],
    ramp: &[usize],
    atlas: &Atlas,
    params: &Params,
    columns: u32,
    rows: u32,
) -> RgbaImage {
    let cell_w = atlas.cell_w;
    let cell_h = atlas.cell_h;
    let mut image = RgbaImage::new(columns * cell_w, rows * cell_h);
    fill_background(&mut image, params.background);
    let raw = image.as_mut();
    let stride = (columns * cell_w) as usize * 4;
    let (bg_r, bg_g, bg_b) = params.background.to_f32();

    for row in 0..rows {
        for col in 0..columns {
            let cell = (row * columns + col) as usize;
            let glyph = &atlas.glyphs[ramp[chosen[cell] as usize]];
            let (red, green, blue) = glyph_color(samples[cell], params);
            let origin_x = col * cell_w;
            let origin_y = row * cell_h;
            for py in 0..cell_h {
                let dest_row = (origin_y + py) as usize * stride;
                let mask_row = py * cell_w;
                for px in 0..cell_w {
                    let alpha =
                        glyph.mask[(mask_row + px) as usize] as f32 * (params.weight / 255.0);
                    if alpha <= 0.0 {
                        continue;
                    }
                    let alpha = alpha.min(1.0);
                    let offset = dest_row + (origin_x + px) as usize * 4;
                    raw[offset] = to_u8(bg_r + (red - bg_r) * alpha);
                    raw[offset + 1] = to_u8(bg_g + (green - bg_g) * alpha);
                    raw[offset + 2] = to_u8(bg_b + (blue - bg_b) * alpha);
                    raw[offset + 3] = 255;
                }
            }
        }
    }
    image
}

fn stamp_ramp(atlas: &Atlas, ramp: &[usize], params: &Params) -> RgbaImage {
    let gap = 6u32;
    let count = ramp.len() as u32;
    let width = count * atlas.cell_w + count.saturating_sub(1) * gap;
    let mut image = RgbaImage::new(width.max(1), atlas.cell_h.max(1));
    let ink = ramp_ink(params);
    fill_background(&mut image, params.background);
    let stride = image.width() as usize * 4;
    let raw = image.as_mut();
    let (bg_r, bg_g, bg_b) = params.background.to_f32();
    let (red, green, blue) = ink.to_f32();

    for (index, &glyph_index) in ramp.iter().enumerate() {
        let glyph = &atlas.glyphs[glyph_index];
        let origin_x = index as u32 * (atlas.cell_w + gap);
        for py in 0..atlas.cell_h {
            for px in 0..atlas.cell_w {
                let alpha =
                    glyph.mask[(py * atlas.cell_w + px) as usize] as f32 * (params.weight / 255.0);
                if alpha <= 0.0 {
                    continue;
                }
                let alpha = alpha.min(1.0);
                let offset = py as usize * stride + (origin_x + px) as usize * 4;
                raw[offset] = to_u8(bg_r + (red - bg_r) * alpha);
                raw[offset + 1] = to_u8(bg_g + (green - bg_g) * alpha);
                raw[offset + 2] = to_u8(bg_b + (blue - bg_b) * alpha);
                raw[offset + 3] = 255;
            }
        }
    }
    image
}

fn glyph_color(sample: Sample, params: &Params) -> (f32, f32, f32) {
    match params.color_mode {
        ColorMode::Ink => params.ink.to_f32(),
        ColorMode::Image => saturate(sample.r, sample.g, sample.b, params.saturation),
    }
}

fn ramp_ink(params: &Params) -> Rgb {
    if params.color_mode == ColorMode::Ink {
        params.ink
    } else if params.background.luma() > 0.6 {
        Rgb::new(32, 26, 22)
    } else {
        Rgb::new(236, 228, 214)
    }
}

fn saturate(r: f32, g: f32, b: f32, amount: f32) -> (f32, f32, f32) {
    let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    (
        (luma + (r - luma) * amount).clamp(0.0, 1.0),
        (luma + (g - luma) * amount).clamp(0.0, 1.0),
        (luma + (b - luma) * amount).clamp(0.0, 1.0),
    )
}

fn fill_background(image: &mut RgbaImage, color: Rgb) {
    for pixel in image.as_mut().chunks_mut(4) {
        pixel[0] = color.r;
        pixel[1] = color.g;
        pixel[2] = color.b;
        pixel[3] = 255;
    }
}

fn to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{ColorMode, Preset};
    use image::{Rgba, RgbaImage};

    #[test]
    fn square_cells_keep_the_photo_aspect() {
        assert_eq!(rows_for(200, 100, 40, 10, 10), 20);
    }

    #[test]
    fn halfwidth_cells_keep_the_photo_aspect() {
        // 40 columns of 5×10 cells over a square photo is a square picture.
        assert_eq!(rows_for(100, 100, 40, 5, 10), 20);
    }

    #[test]
    fn gradient_puts_dark_characters_on_the_dark_side() {
        let mut engine = match Engine::open() {
            Ok(engine) => engine,
            Err(err) => {
                eprintln!("skipping render test: {err}");
                return;
            }
        };

        let mut params = Params::default();
        Preset::Paper.apply(&mut params);
        params.columns = 48;
        params.cell_px = 12;
        params.dither = false;
        params.outlines = 0.0;
        params.stretch = 0.0;
        params.contrast = 1.0;
        params.brightness = 0.0;
        params.gamma = 1.0;
        params.weight = 1.0;
        params.color_mode = ColorMode::Ink;

        let gradient = gradient_image(96, 32);
        let rendered = engine
            .render(1, &gradient, &params)
            .expect("render gradient");
        assert_eq!(
            rendered.image.width(),
            rendered.stats.columns * rendered.stats.cell_w
        );
        assert!(rendered.stats.rows >= 1);

        let left = mean_luma(&rendered.image, 0, rendered.image.width() / 4);
        let right = mean_luma(
            &rendered.image,
            rendered.image.width() * 3 / 4,
            rendered.image.width(),
        );
        assert!(
            left > right + 0.08,
            "expected the white side to stay lighter than the black side, left {left} right {right}"
        );
        assert!(
            rendered.characters.chars().count() >= 2,
            "the ramp should contain more than a blank"
        );

        let white = RgbaImage::from_pixel(16, 16, Rgba([255, 255, 255, 255]));
        let black = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 255]));
        let white_art = engine.render(2, &white, &params).unwrap();
        let black_art = engine.render(3, &black, &params).unwrap();
        let white_luma = mean_luma(&white_art.image, 0, white_art.image.width());
        let black_luma = mean_luma(&black_art.image, 0, black_art.image.width());
        assert!(
            black_luma + 0.12 < white_luma,
            "black {black_luma} should be darker than white {white_luma}"
        );
    }

    fn gradient_image(width: u32, height: u32) -> RgbaImage {
        let mut image = RgbaImage::new(width, height);
        for x in 0..width {
            let shade = (255.0 * (1.0 - x as f32 / (width - 1) as f32)).round() as u8;
            for y in 0..height {
                image.put_pixel(x, y, Rgba([shade, shade, shade, 255]));
            }
        }
        image
    }

    fn mean_luma(image: &RgbaImage, x0: u32, x1: u32) -> f32 {
        let mut sum = 0.0f32;
        let mut count = 0u32;
        for y in 0..image.height() {
            for x in x0..x1 {
                let pixel = image.get_pixel(x, y);
                sum += (0.2126 * pixel[0] as f32
                    + 0.7152 * pixel[1] as f32
                    + 0.0722 * pixel[2] as f32)
                    / 255.0;
                count += 1;
            }
        }
        sum / count.max(1) as f32
    }
}
