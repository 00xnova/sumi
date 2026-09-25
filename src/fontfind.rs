//! Locate a Japanese font installed on the machine.

use ab_glyph::FontVec;
use anyhow::{Context, Result, anyhow, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct FontSource {
    pub path: PathBuf,
    pub index: u32,
    pub family: String,
}

const FALLBACKS: &[(&str, u32, &str)] = &[
    (
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        5,
        "Noto Sans Mono CJK JP",
    ),
    (
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        5,
        "Noto Sans Mono CJK JP",
    ),
    (
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        5,
        "Noto Sans Mono CJK JP",
    ),
    (
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        0,
        "Noto Sans CJK JP",
    ),
];

const HELP: &str = "\
Sumi needs a Japanese font and could not find one.
Install Noto Sans CJK, for example:
  sudo pacman -S noto-fonts-cjk
Or point SUMI_FONT at a font file. For a collection, add # and the face index:
  SUMI_FONT=/path/to/NotoSansCJK-Regular.ttc#5";

pub fn locate() -> Result<FontSource> {
    if let Ok(spec) = std::env::var("SUMI_FONT") {
        let source = parse_spec(&spec)?;
        if !source.path.is_file() {
            bail!("{} is not a file\n{HELP}", source.path.display());
        }
        return Ok(source);
    }

    for query in [
        "Noto Sans Mono CJK JP",
        "Noto Sans CJK JP",
        "Source Han Mono",
    ] {
        if let Some(found) = fc_match(query)
            && is_cjk(&found.family)
            && found.path.is_file()
        {
            return Ok(found);
        }
    }

    for (path, index, family) in FALLBACKS {
        if Path::new(path).is_file() {
            return Ok(FontSource {
                path: PathBuf::from(path),
                index: *index,
                family: (*family).to_string(),
            });
        }
    }

    bail!("{HELP}")
}

pub fn load(source: &FontSource) -> Result<FontVec> {
    let bytes = std::fs::read(&source.path)
        .with_context(|| format!("could not read {}", source.path.display()))?;
    FontVec::try_from_vec_and_index(bytes, source.index).map_err(|_| {
        anyhow!(
            "could not read {} as a font (face index {})",
            source.path.display(),
            source.index
        )
    })
}

fn parse_spec(spec: &str) -> Result<FontSource> {
    let spec = spec.trim();
    if spec.is_empty() {
        bail!("SUMI_FONT is empty\n{HELP}");
    }
    let (path, index) = match spec.rsplit_once('#') {
        Some((path, index)) => {
            let index = index
                .parse::<u32>()
                .with_context(|| format!("font face index '{index}' is not a number"))?;
            (path, index)
        }
        None => (spec, 0),
    };
    Ok(FontSource {
        path: PathBuf::from(path),
        index,
        family: Path::new(path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Custom font".to_string()),
    })
}

fn fc_match(query: &str) -> Option<FontSource> {
    let output = Command::new("fc-match")
        .args(["-f", "%{file}\\n%{index}\\n%{family}\\n", query])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let mut lines = text.lines();
    let path = lines.next()?.trim();
    let index = lines.next()?.trim().parse().ok()?;
    let family = short_family(lines.next()?.trim());
    if path.is_empty() || family.is_empty() {
        return None;
    }
    Some(FontSource {
        path: PathBuf::from(path),
        index,
        family,
    })
}

fn short_family(family: &str) -> String {
    family
        .split(',')
        .next()
        .unwrap_or(family)
        .trim()
        .to_string()
}

fn is_cjk(family: &str) -> bool {
    let family = family.to_lowercase();
    [
        "cjk",
        "japanese",
        "gothic",
        "source han",
        "noto sans jp",
        "ipa",
    ]
    .iter()
    .any(|needle| family.contains(needle))
}
