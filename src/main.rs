//! Sumi opens a window by default. `sumi render` writes a PNG or WebP directly.

#![forbid(unsafe_code)]

mod gui;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use sumi::{ColorMode, Engine, Palette, Params, Preset, Style, load_image, save_image};

#[derive(Parser)]
#[command(
    name = "sumi",
    version,
    about = "Turn a picture into Japanese character art",
    long_about = "Sumi turns a picture into Japanese character art.\n\n\
With no arguments it opens a window: drop in a photo, tune the sliders, and save a PNG or WebP.\n\
`sumi render` writes a file without opening the window."
)]
struct Cli {
    /// Image to open in the window.
    #[arg(value_name = "IMAGE")]
    image: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Write character art to a PNG or WebP file.
    Render {
        input: PathBuf,
        output: PathBuf,
        /// Characters across the picture.
        #[arg(long)]
        columns: Option<u32>,
        /// Pixel height of each character in the saved file.
        #[arg(long)]
        cell: Option<u32>,
        /// kanji, kana, or halfwidth.
        #[arg(long)]
        style: Option<String>,
        /// How many distinct characters share the shading.
        #[arg(long)]
        levels: Option<u32>,
        #[arg(long)]
        brightness: Option<f32>,
        #[arg(long)]
        contrast: Option<f32>,
        #[arg(long)]
        gamma: Option<f32>,
        /// Pull flat photos toward a full light-to-dark range, from 0 to 1.
        #[arg(long)]
        stretch: Option<f32>,
        /// Darken edges so shapes read clearly, from 0 to 1.
        #[arg(long)]
        outlines: Option<f32>,
        /// Make strokes heavier. 1 is the natural weight of the font.
        #[arg(long)]
        weight: Option<f32>,
        /// How vivid sampled photo colors are. 1 leaves them unchanged.
        #[arg(long)]
        saturation: Option<f32>,
        /// Swap light and dark.
        #[arg(long)]
        invert: bool,
        /// Keep flat areas of one character instead of softening gradients.
        #[arg(long)]
        flat: bool,
        /// Draw every character in one ink color.
        #[arg(long)]
        mono: bool,
        /// color, paper, screen, or stamp.
        #[arg(long)]
        preset: Option<String>,
    },
}

fn main() {
    if let Err(err) = dispatch() {
        eprintln!("sumi: {err:#}");
        std::process::exit(1);
    }
}

fn dispatch() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Render {
            input,
            output,
            columns,
            cell,
            style,
            levels,
            brightness,
            contrast,
            gamma,
            stretch,
            outlines,
            weight,
            saturation,
            invert,
            flat,
            mono,
            preset,
        }) => render_file(RenderSettings {
            input,
            output,
            columns,
            cell,
            style,
            levels,
            brightness,
            contrast,
            gamma,
            stretch,
            outlines,
            weight,
            saturation,
            invert,
            flat,
            mono,
            preset,
        }),
        None => gui::launch(cli.image).map_err(|err| anyhow!("{err}")),
    }
}

struct RenderSettings {
    input: PathBuf,
    output: PathBuf,
    columns: Option<u32>,
    cell: Option<u32>,
    style: Option<String>,
    levels: Option<u32>,
    brightness: Option<f32>,
    contrast: Option<f32>,
    gamma: Option<f32>,
    stretch: Option<f32>,
    outlines: Option<f32>,
    weight: Option<f32>,
    saturation: Option<f32>,
    invert: bool,
    flat: bool,
    mono: bool,
    preset: Option<String>,
}

fn render_file(settings: RenderSettings) -> Result<()> {
    let image = load_image(&settings.input)?;
    let mut params = Params::default();
    if let Some(name) = settings.preset {
        let preset = Preset::parse(&name).ok_or_else(|| {
            anyhow!("unknown preset '{name}'. Use color, paper, screen, or stamp.")
        })?;
        preset.apply_with(&mut params, Palette::load().as_ref());
    }
    if let Some(columns) = settings.columns {
        params.columns = columns;
    }
    if let Some(cell) = settings.cell {
        params.cell_px = cell;
    }
    if let Some(style) = settings.style {
        params.style = Style::parse(&style)
            .ok_or_else(|| anyhow!("unknown style '{style}'. Use kanji, kana, or halfwidth."))?;
    }
    if let Some(levels) = settings.levels {
        params.levels = levels;
    }
    if let Some(brightness) = settings.brightness {
        params.brightness = brightness;
    }
    if let Some(contrast) = settings.contrast {
        params.contrast = contrast;
    }
    if let Some(gamma) = settings.gamma {
        params.gamma = gamma;
    }
    if let Some(stretch) = settings.stretch {
        params.stretch = stretch;
    }
    if let Some(outlines) = settings.outlines {
        params.outlines = outlines;
    }
    if let Some(weight) = settings.weight {
        params.weight = weight;
    }
    if let Some(saturation) = settings.saturation {
        params.saturation = saturation;
    }
    if settings.invert {
        params.invert = true;
    }
    if settings.flat {
        params.dither = false;
    }
    if settings.mono {
        params.color_mode = ColorMode::Ink;
    }
    params = params.sanitize();

    let mut engine = Engine::open()?;
    let rendered = engine.render(0, &image, &params)?;
    save_image(&settings.output, &rendered.image)?;
    println!(
        "Wrote {}\n{}×{} characters · {}×{} px · {} · {} ms\nLight to dark: {}",
        settings.output.display(),
        rendered.stats.columns,
        rendered.stats.rows,
        rendered.stats.width,
        rendered.stats.height,
        rendered.stats.font,
        rendered.stats.elapsed_ms,
        rendered.characters,
    );
    if rendered.stats.capped {
        println!("The picture was capped so the long side stays within 8192 px.");
    }
    Ok(())
}
