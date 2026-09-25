//! The palette of the Omarchy theme that is applied right now.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::params::Rgb;

/// Colors from `~/.local/state/omarchy/current/theme/colors.toml`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub dark: bool,
    pub accent: Rgb,
    pub background: Rgb,
    pub darker_background: Rgb,
    pub lighter_background: Rgb,
    pub foreground: Rgb,
    pub muted: Rgb,
    pub border: Rgb,
    pub red: Rgb,
}

impl Palette {
    /// Read the theme Omarchy last applied. `None` when that file is missing.
    pub fn load() -> Option<Self> {
        let path = colors_path()?;
        let text = std::fs::read_to_string(path).ok()?;
        Self::parse(&text)
    }

    pub fn modified() -> Option<SystemTime> {
        std::fs::metadata(colors_path()?).ok()?.modified().ok()
    }

    pub fn fallback() -> Self {
        Self::parse(
            r##"
mode = "dark"
accent = "#e68e0d"
muted = "#333333"
background = "#121212"
darker_background = "#090909"
lighter_background = "#1e1e1e"
foreground = "#bebebe"
red = "#D35F5F"
"##,
        )
        .expect("built-in palette")
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut values = std::collections::HashMap::<String, String>::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_matches('"').trim();
            values.insert(key.trim().to_string(), value.to_string());
        }

        let background = color(&values, "background")?;
        let foreground = color(&values, "foreground")?;
        let accent = color(&values, "accent").unwrap_or(foreground);
        let darker_background = color(&values, "darker_background")
            .or_else(|| color(&values, "dark_background"))
            .unwrap_or(background);
        let lighter_background = color(&values, "lighter_background").unwrap_or(background);
        let dark = values
            .get("mode")
            .map(|mode| mode != "light")
            .unwrap_or(true);
        let border = color(&values, "muted").unwrap_or_else(|| mix(background, foreground, 0.22));
        let red = color(&values, "red")
            .or_else(|| color(&values, "bright_red"))
            .unwrap_or(accent);

        Some(Self {
            dark,
            accent,
            background,
            darker_background,
            lighter_background,
            foreground,
            muted: mix(foreground, background, 0.42),
            border,
            red,
        })
    }
}

fn colors_path() -> Option<PathBuf> {
    if let Ok(state) = std::env::var("XDG_STATE_HOME")
        && !state.is_empty()
    {
        let path = Path::new(&state).join("omarchy/current/theme/colors.toml");
        if path.is_file() {
            return Some(path);
        }
    }
    let home = std::env::var("HOME").ok()?;
    let path = Path::new(&home).join(".local/state/omarchy/current/theme/colors.toml");
    path.is_file().then_some(path)
}

fn color(values: &std::collections::HashMap<String, String>, key: &str) -> Option<Rgb> {
    values.get(key).and_then(|value| hex(value))
}

fn hex(value: &str) -> Option<Rgb> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 {
        return None;
    }
    let packed = u32::from_str_radix(value, 16).ok()?;
    Some(Rgb::new(
        (packed >> 16) as u8,
        (packed >> 8) as u8,
        packed as u8,
    ))
}

fn mix(from: Rgb, to: Rgb, toward: f32) -> Rgb {
    let toward = toward.clamp(0.0, 1.0);
    let blend = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * toward).round() as u8;
    Rgb::new(
        blend(from.r, to.r),
        blend(from.g, to.g),
        blend(from.b, to.b),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_omarchy_palette() {
        let palette = Palette::parse(
            r##"
mode = "dark"
accent = "#e68e0d"
muted = "#333333"
background = "#121212"
darker_background = "#090909"
lighter_background = "#1e1e1e"
foreground = "#bebebe"
red = "#D35F5F"
"##,
        )
        .expect("palette");
        assert!(palette.dark);
        assert_eq!(palette.accent, Rgb::new(0xe6, 0x8e, 0x0d));
        assert_eq!(palette.background, Rgb::new(0x12, 0x12, 0x12));
        assert_eq!(palette.border, Rgb::new(0x33, 0x33, 0x33));
        assert_eq!(palette.red, Rgb::new(0xd3, 0x5f, 0x5f));
    }
}
