//! How the client lays text out (decision T237), recorded by StdTest's
//! glyphs and screen probes (`data/camelot-glyphs.txt`,
//! `tests/client/forever-camelot-glyphs.lua`, `…-screens.lua`):
//!
//! - A font renders at a whole pixel size: its size over the screen's
//!   pixel (768 units over the screen's height at the region's effective
//!   scale, UIParent's included), rounded. Sizes that round alike gave the
//!   same glyphs.
//! - A line is its glyphs' advances but the last one's, the last glyph's
//!   width, the kerning of pairs that kern, and one pixel for a shadow
//!   offset to the right (recorded at five screens, offset 1); then times
//!   the pixel, as a single float.
//! - A line's height is the font's at that pixel size.
//!
//! Recorded: FRIZQT, ARIALN (both plain and outlined), skurri and MORPHEUS
//! at 8 to 45 pixels, every printable ASCII character; kerning at FRIZQT 17
//! and ARIALN 20 pixels. Unverified: other fonts (measured as FRIZQT),
//! sizes outside 8 to 45 (the nearest scaled), other characters (as `?`),
//! kerning at other sizes (none), a texture's width in text (none).

use std::collections::HashMap;
use std::sync::OnceLock;

use wowlua_vm::{Value, Vm};

use crate::engine;
use crate::widgets::{self, WidgetId};

/// A font's glyphs at one pixel size, in pixels: the string height, then
/// each printable character's width alone and its advance (32 to 126).
pub struct Row {
    pub height: f64,
    pub widths: Vec<f64>,
    pub advances: Vec<f64>,
}

struct Glyphs {
    rows: HashMap<(String, bool, i64), Row>,
    kern: HashMap<(String, i64, char, char), f64>,
}

fn glyphs() -> &'static Glyphs {
    static GLYPHS: OnceLock<Glyphs> = OnceLock::new();
    GLYPHS.get_or_init(|| {
        let mut rows = HashMap::new();
        let mut kern = HashMap::new();
        for line in include_str!("../data/camelot-glyphs.txt").lines() {
            if line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            let nums =
                |s: &str| -> Vec<f64> { s.split(',').filter_map(|v| v.parse().ok()).collect() };
            match f.as_slice() {
                ["kern", font, px, pair, delta] => {
                    let mut c = pair.chars();
                    if let (Some(a), Some(b), Ok(px), Ok(d)) =
                        (c.next(), c.next(), px.parse(), delta.parse())
                    {
                        kern.insert((font.to_ascii_lowercase(), px, a, b), d);
                    }
                }
                [font, flags, px, height, widths, advances] => {
                    if let (Ok(px), Ok(height)) = (px.parse::<i64>(), height.parse::<f64>()) {
                        rows.insert(
                            (font.to_ascii_lowercase(), *flags == "OUTLINE", px),
                            Row {
                                height,
                                widths: nums(widths),
                                advances: nums(advances),
                            },
                        );
                    }
                }
                _ => {}
            }
        }
        Glyphs { rows, kern }
    })
}

/// A font as a region uses it: its file's name (lower case, without its
/// folder), its size, whether outlined, its shadow's offset to the right.
pub struct Font {
    pub file: String,
    pub size: f64,
    pub outline: bool,
    pub shadow_x: f64,
}

/// The region's font: its own or its font object's.
pub fn font_of(vm: &Vm, id: WidgetId) -> Font {
    let font = widgets::font_chain_ref(vm, id, "Font").unwrap_or_default();
    let text = |v: Option<&Value>| match v {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.heap.str_bytes(*s)).into_owned(),
        _ => String::new(),
    };
    let (file, flags) = (text(font.first()), text(font.get(2)));
    let size = match font.get(1) {
        Some(Value::Num(n)) => *n,
        _ => 0.0,
    };
    let shadow_x =
        match widgets::font_chain_ref(vm, id, "ShadowOffset").and_then(|v| v.first().copied()) {
            Some(Value::Num(x)) => x,
            _ => 0.0,
        };
    let file = file
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    Font {
        file,
        size,
        outline: flags.to_ascii_uppercase().contains("OUTLINE"),
        shadow_x,
    }
}

/// The screen's pixel in the region's own units: 768 over the screen's
/// height at its effective scale, UIParent's scale included unless it
/// ignores its parents' (recorded at five screens).
pub fn pixel(vm: &Vm, id: WidgetId) -> f32 {
    let e = engine(vm);
    // In single floats (the recorded widths' last digit varies by a float
    // step from this, the pixels exact: unverified how the client rounds).
    768.0f32 / (e.screen.height as f32 * widgets::effective_scale(e, id) as f32)
}

/// The font's glyphs at a pixel size, and the factor a size outside the
/// recorded ones is scaled by.
fn row(font: &Font, px: i64) -> (&'static Row, f64) {
    let g = glyphs();
    let file = if g.rows.contains_key(&(font.file.clone(), font.outline, 12)) {
        font.file.clone()
    } else {
        "frizqt__.ttf".to_string()
    };
    let near = px.clamp(8, 45);
    let row = g
        .rows
        .get(&(file.clone(), font.outline, near))
        .or_else(|| g.rows.get(&(file, false, near)))
        .expect("the recorded fonts' rows");
    (row, px as f64 / near as f64)
}

/// The text a region shows: its escape sequences taken out (`|cAARRGGBB`
/// and `|r` colours, `|H…|h` links' codes, `|T…|t` textures, `|n` a new
/// line, `||` a bar).
pub fn visible(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '|' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('|') => out.push('|'),
            Some('c') => {
                for _ in 0..8 {
                    chars.next();
                }
            }
            Some('r') | Some('h') => {}
            Some('n') => out.push('\n'),
            Some('H') => {
                // The link's code, to its `|h`.
                while let Some(x) = chars.next() {
                    if x == '|' && chars.peek() == Some(&'h') {
                        chars.next();
                        break;
                    }
                }
            }
            Some('T') | Some('A') => {
                // A texture or an atlas, to its `|t` / `|a`.
                while let Some(x) = chars.next() {
                    if x == '|' && matches!(chars.peek(), Some('t') | Some('a')) {
                        chars.next();
                        break;
                    }
                }
            }
            Some(other) => {
                out.push('|');
                out.push(other);
            }
            None => out.push('|'),
        }
    }
    out
}

/// One line's width in pixels at the font's pixel size.
fn line_px(font: &Font, px: i64, line: &str) -> f64 {
    let chars: Vec<char> = line.chars().collect();
    let Some(&last) = chars.last() else {
        return 0.0;
    };
    let (row, scale) = row(font, px);
    let index = |c: char| {
        let i = c as u32;
        if (32..=126).contains(&i) {
            (i - 32) as usize
        } else {
            ('?' as u32 - 32) as usize
        }
    };
    let mut total = 0.0;
    for &c in &chars[..chars.len() - 1] {
        total += row.advances[index(c)] * scale;
    }
    total += row.widths[index(last)] * scale;
    let g = glyphs();
    for pair in chars.windows(2) {
        if let Some(d) = g.kern.get(&(font.file.clone(), px, pair[0], pair[1])) {
            total += d;
        }
    }
    if font.shadow_x > 0.0 {
        total += 1.0;
    }
    total.round()
}

/// The pixel size a font renders at in a region.
fn pixel_size(font: &Font, pixel: f32) -> i64 {
    (font.size as f32 / pixel).round() as i64
}

/// The width of `text`'s widest line in the region, in its units (a
/// single float, as the client's).
pub fn width(vm: &Vm, id: WidgetId, text: &str) -> f64 {
    let font = font_of(vm, id);
    if font.size <= 0.0 {
        return 0.0;
    }
    let pixel = pixel(vm, id);
    let px = pixel_size(&font, pixel);
    let widest = visible(text)
        .split('\n')
        .map(|line| line_px(&font, px, line))
        .fold(0.0, f64::max);
    f64::from(widest as f32 * pixel)
}

/// A line's height in the region, in its units.
pub fn line_height(vm: &Vm, id: WidgetId) -> f64 {
    let font = font_of(vm, id);
    if font.size <= 0.0 {
        return 0.0;
    }
    let pixel = pixel(vm, id);
    let px = pixel_size(&font, pixel);
    let (row, scale) = row(&font, px);
    f64::from((row.height * scale).round() as f32 * pixel)
}

/// The visible text's lines, each wrapped by words at `max` units when it
/// is wider (none: unwrapped). A word wider than `max` keeps a line of its
/// own (truncated), or with `SetNonSpaceWrap(true)` breaks between its
/// characters (unverified in the client).
pub fn wrapped_lines(vm: &Vm, id: WidgetId, text: &str, max: Option<f64>) -> Vec<String> {
    let font = font_of(vm, id);
    let pixel = pixel(vm, id);
    let px = pixel_size(&font, pixel);
    let props = engine(vm).world.widgets[id.0].props;
    let non_space = crate::widgets::stored_ref(vm, props, "NonSpaceWrap")
        .and_then(|v| v.first().copied())
        .is_some_and(|v| v.truthy());
    let fits = |s: &str, max: f64| f64::from(line_px(&font, px, s) as f32 * pixel) <= max + 1e-3;
    let mut out = Vec::new();
    for line in visible(text).split('\n') {
        let Some(max) = max.filter(|m| *m > 0.0 && font.size > 0.0) else {
            out.push(line.to_string());
            continue;
        };
        let mut current = String::new();
        for word in line.split(' ') {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if current.is_empty() || fits(&candidate, max) {
                current = candidate;
            } else {
                out.push(std::mem::take(&mut current));
                current = word.to_string();
            }
            if non_space && !fits(&current, max) {
                // Break the word between characters, each line at least one.
                let mut piece = String::new();
                for ch in current.clone().chars() {
                    let longer = format!("{piece}{ch}");
                    if !piece.is_empty() && !fits(&longer, max) {
                        out.push(std::mem::take(&mut piece));
                        piece = ch.to_string();
                    } else {
                        piece = longer;
                    }
                }
                current = piece;
            }
        }
        out.push(current);
    }
    out
}

/// The character index (1-based) at `x` units on a one-line text, and
/// whether `x` is within it.
pub fn index_at(vm: &Vm, id: WidgetId, text: &str, x: f64) -> (usize, bool) {
    let font = font_of(vm, id);
    let pixel = pixel(vm, id);
    let px = pixel_size(&font, pixel);
    let chars: Vec<char> = visible(text).chars().collect();
    for i in 0..chars.len() {
        let upto: String = chars[..=i].iter().collect();
        if x < f64::from(line_px(&font, px, &upto) as f32 * pixel) {
            return (i + 1, x >= 0.0);
        }
    }
    (chars.len().max(1), false)
}

/// Where a line's glyphs go, as the client measures them: the font's
/// pixel size, the line height in pixels, and each character's left edge
/// in pixels from the line's start (the renderer's text, spec debugger
/// §18.9).
pub struct GlyphLine {
    pub px: i64,
    pub line_height_px: f64,
    pub xs: Vec<f64>,
    pub width_px: f64,
}

pub fn glyph_line(vm: &Vm, id: WidgetId, line: &str) -> GlyphLine {
    let font = font_of(vm, id);
    let pixel = pixel(vm, id);
    let px = pixel_size(&font, pixel).max(1);
    let (row, scale) = row(&font, px);
    let g = glyphs();
    let chars: Vec<char> = line.chars().collect();
    let index = |c: char| {
        let i = c as u32;
        if (32..=126).contains(&i) {
            (i - 32) as usize
        } else {
            ('?' as u32 - 32) as usize
        }
    };
    let mut xs = Vec::with_capacity(chars.len());
    let mut x = 0.0;
    for (i, &c) in chars.iter().enumerate() {
        xs.push(x);
        x += row.advances[index(c)] * scale;
        if let Some(&next) = chars.get(i + 1)
            && let Some(d) = g.kern.get(&(font.file.clone(), px, c, next))
        {
            x += d;
        }
    }
    GlyphLine {
        px,
        line_height_px: (row.height * scale).round(),
        xs,
        width_px: line_px(&font, px, line),
    }
}
