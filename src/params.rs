//! Knobs shared by the window and the `sumi render` command.

use serde::{Deserialize, Serialize};

/// A color stored as sRGB bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_f32(self) -> (f32, f32, f32) {
        (
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
        )
    }

    pub fn luma(self) -> f32 {
        let (r, g, b) = self.to_f32();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }
}

/// Which characters are allowed to shade the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Style {
    /// Kana plus kanji, so the dark end of the picture has dense characters.
    Kanji,
    /// Hiragana and katakana only.
    Kana,
    /// Halfwidth katakana, the classic terminal look.
    Halfwidth,
}

impl Style {
    pub fn label(self) -> &'static str {
        match self {
            Self::Kanji => "Kanji",
            Self::Kana => "Kana",
            Self::Halfwidth => "Halfwidth",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "kanji" => Some(Self::Kanji),
            "kana" => Some(Self::Kana),
            "half" | "halfwidth" | "hankaku" => Some(Self::Halfwidth),
            _ => None,
        }
    }
}

/// Where a character's color comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorMode {
    /// Each character takes the average color of the photo underneath it.
    Image,
    /// Every character is drawn in one ink color.
    Ink,
}

/// A group of color settings that can be applied without touching detail sliders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Color,
    Paper,
    Screen,
    Stamp,
}

impl Preset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Color => "Color",
            Self::Paper => "Paper",
            Self::Screen => "Screen",
            Self::Stamp => "Stamp",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "color" => Some(Self::Color),
            "paper" => Some(Self::Paper),
            "screen" => Some(Self::Screen),
            "stamp" => Some(Self::Stamp),
            _ => None,
        }
    }

    pub fn all() -> [Self; 4] {
        [Self::Color, Self::Paper, Self::Screen, Self::Stamp]
    }

    /// Replace ink, background, and color mode. Detail sliders stay as they are.
    pub fn apply(self, params: &mut Params) {
        self.apply_with(params, None);
    }

    /// Like [`Self::apply`], using the current Omarchy palette when one is passed in.
    pub fn apply_with(self, params: &mut Params, palette: Option<&crate::Palette>) {
        params.invert = false;
        match self {
            Self::Color => {
                params.color_mode = ColorMode::Image;
                if let Some(palette) = palette {
                    params.background = palette.background;
                    params.ink = palette.foreground;
                } else {
                    params.background = Rgb::new(16, 14, 13);
                    params.ink = Rgb::new(242, 236, 226);
                }
            }
            Self::Paper => {
                params.color_mode = ColorMode::Ink;
                if let Some(palette) = palette.filter(|palette| !palette.dark) {
                    params.background = palette.background;
                    params.ink = palette.foreground;
                } else {
                    params.background = Rgb::new(245, 240, 230);
                    params.ink = Rgb::new(32, 26, 22);
                }
            }
            Self::Screen => {
                params.color_mode = ColorMode::Ink;
                if let Some(palette) = palette {
                    params.background = palette.background;
                    params.ink = palette.foreground;
                } else {
                    params.background = Rgb::new(14, 15, 18);
                    params.ink = Rgb::new(232, 232, 228);
                }
            }
            Self::Stamp => {
                params.color_mode = ColorMode::Ink;
                if let Some(palette) = palette {
                    params.ink = palette.red;
                    params.background = palette.background;
                } else {
                    params.background = Rgb::new(246, 239, 226);
                    params.ink = Rgb::new(176, 42, 36);
                }
            }
        }
    }

    pub fn matching(params: &Params) -> Option<Self> {
        Self::matching_with(params, None)
    }

    pub fn matching_with(params: &Params, palette: Option<&crate::Palette>) -> Option<Self> {
        Self::all().into_iter().find(|preset| {
            let mut trial = *params;
            preset.apply_with(&mut trial, palette);
            trial.color_mode == params.color_mode
                && trial.ink == params.ink
                && trial.background == params.background
                && trial.invert == params.invert
        })
    }
}

/// Everything that changes the picture besides the source photo.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    pub columns: u32,
    pub cell_px: u32,
    pub style: Style,
    pub levels: u32,
    pub brightness: f32,
    pub contrast: f32,
    pub gamma: f32,
    /// 0 leaves the photo's tones alone. 1 stretches them to fill light and dark.
    pub stretch: f32,
    /// How much edges push characters toward the dark end of the ramp.
    pub outlines: f32,
    /// Multiplier on character coverage. Higher values make heavier strokes.
    pub weight: f32,
    /// Saturation of colors sampled from the photo. 1 is unchanged.
    pub saturation: f32,
    /// 0 lets the brightest tones use an empty square. Higher keeps a written character there.
    #[serde(default = "default_character_floor")]
    pub character_floor: f32,
    /// 1 uses the heaviest character for black. Lower stops before the ink turns into a solid mass.
    #[serde(default = "default_character_ceiling")]
    pub character_ceiling: f32,
    /// Let the brightest cells be an empty square.
    #[serde(default)]
    pub allow_blank: bool,
    /// Let the darkest cells be a filled geometric block.
    #[serde(default)]
    pub solid_blocks: bool,
    pub invert: bool,
    pub dither: bool,
    pub color_mode: ColorMode,
    pub ink: Rgb,
    pub background: Rgb,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            columns: 96,
            cell_px: 22,
            style: Style::Kanji,
            levels: 34,
            brightness: 0.0,
            contrast: 1.08,
            gamma: 1.0,
            stretch: 0.82,
            outlines: 0.22,
            weight: 1.25,
            saturation: 1.12,
            character_floor: default_character_floor(),
            character_ceiling: default_character_ceiling(),
            allow_blank: false,
            solid_blocks: false,
            invert: false,
            dither: true,
            color_mode: ColorMode::Image,
            ink: Rgb::new(242, 236, 226),
            background: Rgb::new(16, 14, 13),
        }
    }
}

impl Params {
    /// Pull slider and file values back into the range the renderer accepts.
    pub fn sanitize(mut self) -> Self {
        self.columns = self.columns.clamp(8, 320);
        self.cell_px = self.cell_px.clamp(8, 72);
        self.levels = self.levels.clamp(4, 96);
        self.brightness = finite(self.brightness, 0.0).clamp(-0.5, 0.5);
        self.contrast = finite(self.contrast, 1.0).clamp(0.3, 2.5);
        self.gamma = finite(self.gamma, 1.0).clamp(0.3, 2.8);
        self.stretch = finite(self.stretch, 0.0).clamp(0.0, 1.0);
        self.outlines = finite(self.outlines, 0.0).clamp(0.0, 1.5);
        self.weight = finite(self.weight, 1.0).clamp(0.4, 2.4);
        self.saturation = finite(self.saturation, 1.0).clamp(0.0, 2.2);
        self.character_floor =
            finite(self.character_floor, default_character_floor()).clamp(0.0, 0.85);
        self.character_ceiling =
            finite(self.character_ceiling, default_character_ceiling()).clamp(0.25, 1.0);
        if self.character_ceiling < self.character_floor + 0.05 {
            self.character_ceiling = (self.character_floor + 0.05).min(1.0);
        }
        self
    }
}

fn default_character_floor() -> f32 {
    0.34
}

fn default_character_ceiling() -> f32 {
    1.0
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_preset_keeps_detail_settings() {
        let mut params = Params {
            columns: 120,
            levels: 20,
            ..Params::default()
        };
        Preset::Paper.apply(&mut params);
        assert_eq!(params.columns, 120);
        assert_eq!(params.levels, 20);
        assert_eq!(params.color_mode, ColorMode::Ink);
        assert_eq!(Preset::matching(&params), Some(Preset::Paper));
    }

    #[test]
    fn sanitize_repairs_bad_numbers() {
        let params = Params {
            columns: 0,
            gamma: f32::NAN,
            weight: 80.0,
            ..Params::default()
        }
        .sanitize();
        assert_eq!(params.columns, 8);
        assert_eq!(params.gamma, 1.0);
        assert_eq!(params.weight, 2.4);
    }
}
