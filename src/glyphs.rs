//! Measure Japanese glyphs and build a ramp from empty paper to solid ink.

use ab_glyph::{Font, FontVec, GlyphId, PxScale, point};
use anyhow::{Result, bail};

use crate::params::Style;

/// Scale-free measurements so any export size can reuse one probe of the font.
#[derive(Clone, Copy, Debug)]
pub struct FontMetrics {
    /// Ink height divided by the fullwidth advance. Near 1 for a square kanji cell.
    pub crop_over_full: f32,
    /// Halfwidth advance divided by the fullwidth advance.
    pub half_over_full: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct FontLayout {
    pub metrics: FontMetrics,
    /// Baseline position inside the cropped cell, 0 at the top and 1 at the bottom.
    pub baseline_in_crop: f32,
    /// `PxScale` (line height) that makes the fullwidth advance one pixel wide.
    pub line_scale_per_advance_px: f32,
}

pub struct Glyph {
    pub ch: char,
    pub coverage: f32,
    pub score: f32,
    pub blank: bool,
    /// A filled or empty geometric box, not a written character.
    pub shape: bool,
    pub mask: Vec<u8>,
}

pub struct RampQuery {
    pub levels: usize,
    pub allow_blank: bool,
    pub allow_shapes: bool,
}

pub struct Atlas {
    pub cell_w: u32,
    pub cell_h: u32,
    pub glyphs: Vec<Glyph>,
}

/// Characters used to find how much of the em square the ink actually occupies.
/// CJK fonts add empty leading above and below the strokes. The picture crops
/// that leading away so rows of characters sit against each other.
const PROBE: &str = "あア一ー、。・っッ口墨鬱龘■ﾊｱﾝﾞﾟ･ｦ";

/// Common kanji spread from a few strokes to very dense ones.
const KANJI: &str = "\
一乙丁七人入八九刀力十下三上丈久亡凡刃千口土士夕大女子寸小山川工己已干弓才\
不中予互五井仁今介仏元公内円分切勿化匹午反友太天少引心戸手支文方日木欠止比毛氏水火父片牛犬王\
世主以仕他付代令兄写冬処出加功包北半占去古句可台右号司各合同名向回因団在地坂多好字存宅安寺対局屋岩島州左市布平年式当形役径忍志忘応快念怒思急性怪恋恐息悪悲情想意愛感成我戦所持打払技投折抜抱招拝拾指振捕改放敗散数料新族早明昔星春昼時晩景晴暖暗暮曲書月有本札材村来東林果枠柄染査業楽樹橋機次歌正歩歯歴死残段母毎気氷永求池決沈没油治法波注泳海消液深混清済減温測満源溶演漢潔潮点無然熱燃版物特状独率玉班球理生用田由甲申男町画界畑留略番異疲病発登白百的盤目直相省眼着矢知石研破確示社礼祖祝神票秋科秒秘移程税種穀穴究空窓立章童端競竹笑第筆等答策算管箱節築米粉精糖糸系紀約紅納純紙級素細終組経結給統絵絶綿総緑線編練県縦縮績織罪置羊美群義羽翌習老考者耐耳聞職聴肉肖肝肥育肺胃背胸能脈脳脚脱腸腹自至致興船般色芋芝花若苦英茶草荷菊菜華落葉著蒸蔵薬虚虫虹蚊蚕血衆行術街衣表裁装裏補製複西要見規視覧親覚観角解言計記訓託訳証評詞試詩話誌認誓語誤説読誰課調談請論諸諦諭謀謡謹識警議譲豊象貝負財貧貨責貯貴買貸費貿賀賃資賛質赤走起超足距跡路身車軍転軽較輪輸辛辞農辺込近返迷追退送逆通速造連週進遊達運過道遠適選遺郷部都配酒酔酢酪酵酷酸醒採里重野量金針釣鈍鉄鉛鉢鉱銀銃銅銑鋼録錘錠銭錯錬鏡長門閉開間関閣防附降限院除陸険陽隅際障集雨雪雲零電需震霊青静非面革靴音頂順預領頭題額風飛食飯飲飼飽飾養餓館首香馬駅駆験骨高髪鬼魚鮮鳥鳴鹿麦黄黒黙鼓鼻齢龍亀鷹麟墨桜猫狐熊虹\
龘鬱鸞鷹鑑驚魔";

pub fn probe_layout(font: &FontVec) -> Result<FontLayout> {
    let line = font.height_unscaled();
    if line <= 1.0 {
        bail!("this font has no line height");
    }
    let full_units = positive_advance(font, 'あ').unwrap_or(font.units_per_em().unwrap_or(line));
    let half_units = positive_advance(font, 'ｱ').unwrap_or(full_units * 0.5);

    // ab_glyph's pixel scale is the line height (ascent − descent), not the em square.
    let scale = 240.0f32;
    let advance_px = full_units * (scale / line);
    let canvas_w = advance_px.ceil().max(8.0) as u32 + 8;
    let canvas_h = (scale * 2.4).ceil() as u32;
    let baseline = scale;
    let mut canvas = Canvas::new(canvas_w, canvas_h);
    for ch in PROBE.chars() {
        paint_glyph(font, ch, scale, 0.0, baseline, &mut canvas);
    }
    if !canvas.any {
        bail!("the font drew no Japanese ink. Try another face of the font.");
    }

    // One pixel of slack at the probe size, so antialiased edges are not cut off.
    let pad = 2i32;
    let min_y = (canvas.min_y - pad).max(0) as f32;
    let max_y = (canvas.max_y + pad).min(canvas_h as i32 - 1) as f32;
    let crop_height = (max_y - min_y + 1.0).max(1.0);

    Ok(FontLayout {
        metrics: FontMetrics {
            crop_over_full: crop_height / advance_px,
            half_over_full: (half_units / full_units).clamp(0.3, 0.7),
        },
        baseline_in_crop: (baseline - min_y) / crop_height,
        line_scale_per_advance_px: line / full_units,
    })
}

pub fn cell_size(style: Style, cell_px: u32, metrics: &FontMetrics) -> (u32, u32) {
    let cell_px = cell_px.max(4);
    let full_px = cell_px as f32 / metrics.crop_over_full.max(0.05);
    let width = match style {
        Style::Halfwidth => full_px * metrics.half_over_full,
        Style::Kana | Style::Kanji => full_px,
    };
    (width.round().max(1.0) as u32, cell_px)
}

pub fn build_atlas(
    font: &FontVec,
    layout: &FontLayout,
    style: Style,
    cell_px: u32,
) -> Result<Atlas> {
    let (cell_w, cell_h) = cell_size(style, cell_px, &layout.metrics);
    let full_px = cell_h as f32 / layout.metrics.crop_over_full.max(0.05);
    let scale = full_px * layout.line_scale_per_advance_px;
    let baseline = layout.baseline_in_crop * cell_h as f32;
    let blank = blank_char(style);

    let mut glyphs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for ch in pool(style) {
        let id = font.glyph_id(ch);
        let is_blank = ch == blank;
        if !is_blank && (id.0 == 0 || !seen.insert(id.0)) {
            continue;
        }
        let mask = if is_blank {
            vec![0u8; (cell_w * cell_h) as usize]
        } else {
            let mut canvas = Canvas::new(cell_w, cell_h);
            paint_glyph(font, ch, scale, 0.0, baseline, &mut canvas);
            canvas.px
        };
        let coverage = coverage_of(&mask);
        if !is_blank && coverage < 0.004 {
            continue;
        }
        let uniformity = uniformity_of(&mask, cell_w, cell_h);
        let bonus = if is_blank { 1.5 } else { category_bonus(ch) };
        glyphs.push(Glyph {
            ch,
            coverage,
            score: uniformity + bonus,
            blank: is_blank,
            shape: is_shape(ch),
            mask,
        });
    }

    if glyphs.len() < 2 {
        bail!("the font did not contain enough Japanese characters to shade a picture");
    }
    glyphs.sort_by(|a, b| a.coverage.total_cmp(&b.coverage));
    Ok(Atlas {
        cell_w,
        cell_h,
        glyphs,
    })
}

pub fn select_ramp(glyphs: &[Glyph], query: RampQuery) -> Vec<usize> {
    if glyphs.is_empty() {
        return Vec::new();
    }
    let mut eligible: Vec<usize> = (0..glyphs.len())
        .filter(|&index| {
            let glyph = &glyphs[index];
            if glyph.blank && !query.allow_blank {
                return false;
            }
            if glyph.shape && !query.allow_shapes {
                return false;
            }
            true
        })
        .collect();
    if eligible.len() < 2 {
        eligible = (0..glyphs.len())
            .filter(|&index| !glyphs[index].blank)
            .collect();
    }
    if eligible.len() < 2 {
        eligible = (0..glyphs.len()).collect();
    }
    eligible.sort_by(|&a, &b| glyphs[a].coverage.total_cmp(&glyphs[b].coverage));
    let levels = query.levels.clamp(2, eligible.len());
    let lo = glyphs[eligible[0]].coverage;
    let hi = glyphs[*eligible.last().unwrap()].coverage;
    let mut used = vec![false; glyphs.len()];
    let mut chosen = Vec::with_capacity(levels);

    for step in 0..levels {
        let target = if hi <= lo {
            lo
        } else {
            lo + (hi - lo) * step as f32 / (levels as f32 - 1.0)
        };
        let window = ((hi - lo) / levels as f32).max(0.001) * 0.8;
        let mut best: Option<usize> = None;
        let mut best_key = f32::MIN;
        for &index in &eligible {
            if used[index] {
                continue;
            }
            let distance = (glyphs[index].coverage - target).abs();
            if distance > window {
                continue;
            }
            let key = glyphs[index].score - distance * 0.35;
            if key > best_key {
                best_key = key;
                best = Some(index);
            }
        }
        let index = best.unwrap_or_else(|| {
            eligible
                .iter()
                .copied()
                .filter(|index| !used[*index])
                .min_by(|&a, &b| {
                    (glyphs[a].coverage - target)
                        .abs()
                        .total_cmp(&(glyphs[b].coverage - target).abs())
                })
                .unwrap_or(eligible[0])
        });
        used[index] = true;
        chosen.push(index);
    }

    chosen.sort_by(|&a, &b| glyphs[a].coverage.total_cmp(&glyphs[b].coverage));
    chosen
}

fn is_shape(ch: char) -> bool {
    matches!(
        ch,
        '■' | '□'
            | '●'
            | '○'
            | '◆'
            | '◇'
            | '▲'
            | '△'
            | '▼'
            | '▽'
            | '★'
            | '☆'
            | '〇'
            | '◯'
            | '◎'
            | '▪'
            | '▫'
            | '⬛'
            | '⬜'
    )
}

fn pool(style: Style) -> Vec<char> {
    let mut chars = Vec::new();
    match style {
        Style::Halfwidth => {
            chars.push(' ');
            chars.extend('\u{FF61}'..='\u{FF9F}');
        }
        Style::Kana | Style::Kanji => {
            chars.push('\u{3000}');
            chars.extend("、。・ー〜～ヽヾゝゞ゛゜´｀＾々〆〇".chars());
            chars.extend('\u{3041}'..='\u{3096}');
            chars.extend('\u{30A1}'..='\u{30FF}');
            if style == Style::Kanji {
                chars.extend(KANJI.chars());
                chars.push('■');
            }
        }
    }
    chars.sort_unstable();
    chars.dedup();
    chars
}

fn blank_char(style: Style) -> char {
    match style {
        Style::Halfwidth => ' ',
        Style::Kana | Style::Kanji => '\u{3000}',
    }
}

fn category_bonus(ch: char) -> f32 {
    match ch as u32 {
        0x30A0..=0x30FF | 0xFF61..=0xFF9F => 0.24,
        0x3040..=0x309F => 0.20,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF => 0.12,
        0x3000..=0x303F => 0.08,
        _ => 0.0,
    }
}

fn coverage_of(mask: &[u8]) -> f32 {
    if mask.is_empty() {
        return 0.0;
    }
    let sum: u64 = mask.iter().map(|value| *value as u64).sum();
    sum as f32 / (mask.len() as f32 * 255.0)
}

fn uniformity_of(mask: &[u8], width: u32, height: u32) -> f32 {
    if width < 4 || height < 4 {
        return 1.0;
    }
    let block_w = width / 4;
    let block_h = height / 4;
    let mut means = [0.0f32; 16];
    for by in 0..4u32 {
        for bx in 0..4u32 {
            let mut sum = 0u32;
            for y in by * block_h..(by + 1) * block_h {
                for x in bx * block_w..(bx + 1) * block_w {
                    sum += mask[(y * width + x) as usize] as u32;
                }
            }
            means[(by * 4 + bx) as usize] = sum as f32 / (block_w * block_h) as f32 / 255.0;
        }
    }
    let mean = means.iter().sum::<f32>() / 16.0;
    let variance = means
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f32>()
        / 16.0;
    1.0 / (1.0 + variance * 12.0)
}

fn positive_advance(font: &FontVec, ch: char) -> Option<f32> {
    let id = font.glyph_id(ch);
    if id.0 == 0 {
        return None;
    }
    let advance = font.h_advance_unscaled(id);
    if advance.is_finite() && advance > 1.0 {
        Some(advance)
    } else {
        None
    }
}

struct Canvas {
    w: u32,
    h: u32,
    px: Vec<u8>,
    min_y: i32,
    max_y: i32,
    any: bool,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            px: vec![0; (w * h) as usize],
            min_y: i32::MAX,
            max_y: i32::MIN,
            any: false,
        }
    }
}

fn paint_glyph(
    font: &FontVec,
    ch: char,
    em_px: f32,
    origin_x: f32,
    baseline: f32,
    canvas: &mut Canvas,
) {
    let id: GlyphId = font.glyph_id(ch);
    if id.0 == 0 {
        return;
    }
    let glyph = id.with_scale_and_position(PxScale::from(em_px), point(origin_x, baseline));
    let Some(outlined) = font.outline_glyph(glyph) else {
        return;
    };
    let bounds = outlined.px_bounds();
    outlined.draw(|x, y, coverage| {
        let px = bounds.min.x.floor() as i32 + x as i32;
        let py = bounds.min.y.floor() as i32 + y as i32;
        if px < 0 || py < 0 || px as u32 >= canvas.w || py as u32 >= canvas.h {
            return;
        }
        let index = (py as u32 * canvas.w + px as u32) as usize;
        let value = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
        if value > canvas.px[index] {
            canvas.px[index] = value;
        }
        if value > 16 {
            canvas.any = true;
            canvas.min_y = canvas.min_y.min(py);
            canvas.max_y = canvas.max_y.max(py);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fontfind;
    use crate::params::Style;

    #[test]
    fn kanji_cells_are_nearly_square_and_glyphs_sit_inside_them() {
        let source = fontfind::locate().expect("font");
        let font = fontfind::load(&source).expect("load");
        let layout = probe_layout(&font).expect("layout");
        let (width, height) = cell_size(Style::Kanji, 48, &layout.metrics);
        let ratio = width as f32 / height as f32;
        assert!(
            (0.85..=1.2).contains(&ratio),
            "cell {width}x{height} ratio {ratio}, metrics {:?}",
            layout.metrics
        );

        let atlas = build_atlas(&font, &layout, Style::Kanji, 48).expect("atlas");
        let a = atlas
            .glyphs
            .iter()
            .find(|glyph| glyph.ch == 'あ')
            .expect("あ");
        let block = atlas
            .glyphs
            .iter()
            .find(|glyph| glyph.ch == '■')
            .expect("■");
        assert!(
            a.coverage > 0.08 && a.coverage < 0.55,
            "あ coverage {}",
            a.coverage
        );
        assert!(block.coverage > 0.45, "■ coverage {}", block.coverage);
        let center = vertical_center(&a.mask, atlas.cell_w, atlas.cell_h);
        assert!(
            (0.3..=0.7).contains(&center),
            "あ should sit in the middle of the cell, center {center}"
        );
    }

    fn vertical_center(mask: &[u8], width: u32, height: u32) -> f32 {
        let mut weight = 0.0f32;
        let mut moment = 0.0f32;
        for y in 0..height {
            for x in 0..width {
                let value = mask[(y * width + x) as usize] as f32;
                weight += value;
                moment += value * (y as f32 + 0.5);
            }
        }
        if weight <= 0.0 {
            return 0.0;
        }
        moment / weight / height as f32
    }

    #[test]
    fn default_ramp_uses_written_characters() {
        let glyphs = [
            glyph('　', 0.0, true, false),
            glyph('こ', 0.12, false, false),
            glyph('目', 0.34, false, false),
            glyph('鷹', 0.62, false, false),
            glyph('■', 0.94, false, true),
        ];
        let ramp = select_ramp(
            &glyphs,
            RampQuery {
                levels: 4,
                allow_blank: false,
                allow_shapes: false,
            },
        );
        let chars: String = ramp.iter().map(|&index| glyphs[index].ch).collect();
        assert!(!chars.contains('　'), "{chars}");
        assert!(!chars.contains('■'), "{chars}");
        assert!(chars.contains('こ'), "{chars}");
        assert!(chars.contains('鷹'), "{chars}");
    }

    fn glyph(ch: char, coverage: f32, blank: bool, shape: bool) -> Glyph {
        Glyph {
            ch,
            coverage,
            score: 1.0,
            blank,
            shape,
            mask: Vec::new(),
        }
    }
}
