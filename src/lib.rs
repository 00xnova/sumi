//! Turn a picture into Japanese character art.

#![forbid(unsafe_code)]

mod fontfind;
mod glyphs;
mod io;
mod omarchy;
mod params;
mod render;

pub use io::{fit_edge, load_image, save_image};
pub use omarchy::Palette;
pub use params::{ColorMode, Params, Preset, Rgb, Style};
pub use render::{Engine, FontMetrics, GridSpec, Rendered, Stats, output_size};
