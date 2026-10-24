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
//! at 8 to 45 pixels, every printable ASCII character; kerning at FRIZQT 10, 14,
//! 17, 20 and ARIALN 10, 12, 16, 20 pixels (16 pairs). Unverified: other fonts (measured as FRIZQT),
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

/// The text a region shows: its escape sequences taken out (`|cAARRGGBB`,
/// `|cnNAME:` (a named colour) and `|r` colours, `|H…|h` links' codes, `|T…|t` textures, `|n` a new
/// line, `||` a bar, `|4one:many;` the word for the number before it).
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
            // A named colour (`|cnNAME:`), to its colon.
            Some('c') if chars.peek() == Some(&'n') => {
                for x in chars.by_ref() {
                    if x == ':' {
                        break;
                    }
                }
            }
            Some('c') => {
                for _ in 0..8 {
                    chars.next();
                }
            }
            Some('r') | Some('h') => {}
            Some('n') => out.push('\n'),
            Some('4') => {
                let spec: String = chars.by_ref().take_while(|&x| x != ';').collect();
                out.push_str(plural(&out, &spec));
            }
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
                // A texture or an atlas, to its `|t` / `|a`: one character
                // standing for it, its size in it ([`inline_char`]).
                let mut spec = String::new();
                while let Some(x) = chars.next() {
                    if x == '|' && matches!(chars.peek(), Some('t') | Some('a')) {
                        chars.next();
                        break;
                    }
                    spec.push(x);
                }
                out.push(inline_char(inline_spec(&spec).1));
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

/// The text as a log shows it (the debugger's console): what [`visible`]
/// shows, its inline textures left out.
pub fn plain(text: &str) -> String {
    visible(text)
        .chars()
        .filter(|&c| inline_height(c).is_none())
        .collect()
}

/// The form `|4one:many;` takes after `before`: the first for the number 1
/// written last (spaces between), else the second, as the client's chat
/// writes "1 day", "2 days" (`TIME_PLAYED_TOTAL`, `WHO_NUM_RESULTS`).
pub(crate) fn plural<'a>(before: &str, spec: &'a str) -> &'a str {
    let (one, many) = spec.split_once(':').unwrap_or((spec, spec));
    let digits: String = before
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if digits == "1" { one } else { many }
}

/// The first character standing for an inline texture or atlas in laid-out
/// text (a private use plane's); the texture's height in UI units is the
/// character's offset from it, 0 for the text's own height.
const INLINE_BASE: u32 = 0xF0000;

/// The character standing for an inline texture `height` UI units high (0:
/// the text's height).
pub fn inline_char(height: u32) -> char {
    char::from_u32(INLINE_BASE + height.min(1023)).unwrap_or('\u{F0000}')
}

/// The height an inline texture's character stands for, if it is one.
pub fn inline_height(c: char) -> Option<u32> {
    let n = c as u32;
    (INLINE_BASE..INLINE_BASE + 1024)
        .contains(&n)
        .then(|| n - INLINE_BASE)
}

/// An inline texture's or atlas's escape body (`Interface\Icons\X:16:16`,
/// `atlas:0:0`): its file or atlas and its height in UI units (0: the
/// text's), its width the height unless given.
pub fn inline_spec(spec: &str) -> (String, u32, u32) {
    // An atlas's escape has a colon before its name (`|A:name:h:w|a`).
    let spec = spec.strip_prefix(':').unwrap_or(spec);
    let mut parts = spec.split(':');
    let file = parts.next().unwrap_or("").to_string();
    let num = |p: Option<&str>| {
        p.and_then(|v| v.trim().parse::<f64>().ok())
            .map(|v| v.max(0.0) as u32)
    };
    let height = num(parts.next()).unwrap_or(0);
    let width = num(parts.next()).filter(|w| *w > 0).unwrap_or(height);
    (file, height, width)
}

/// An inline texture's advance in a font's pixels: its height's (the font's
/// height for 0).
fn inline_advance(font: &Font, px: i64, height: u32) -> f64 {
    if height == 0 || font.size <= 0.0 {
        px as f64
    } else {
        f64::from(height) * px as f64 / font.size
    }
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
    let advance = |c: char| match inline_height(c) {
        Some(h) => inline_advance(font, px, h),
        None => row.advances[index(c)] * scale,
    };
    for &c in &chars[..chars.len() - 1] {
        total += advance(c);
    }
    total += match inline_height(last) {
        Some(h) => inline_advance(font, px, h),
        None => row.widths[index(last)] * scale,
    };
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

/// A line's advance in pixels: each glyph's advance (the last one's too)
/// and the kerning of pairs that kern. What a line must fit the width in.
fn advance_px(font: &Font, px: i64, chars: &[char]) -> f64 {
    let (row, scale) = row(font, px);
    let index = |c: char| {
        let i = c as u32;
        if (32..=126).contains(&i) {
            (i - 32) as usize
        } else {
            ('?' as u32 - 32) as usize
        }
    };
    let mut total: f64 = chars.iter().map(|&c| row.advances[index(c)] * scale).sum();
    let g = glyphs();
    for pair in chars.windows(2) {
        if let Some(d) = g.kern.get(&(font.file.clone(), px, pair[0], pair[1])) {
            total += d;
        }
    }
    total
}

/// How the client breaks one line of text at a width (`fits`), as
/// recorded by the wrap probe (decision T282): at a space run's end or
/// after a hyphen, the line as long as it fits (its trailing spaces left
/// out); a word wider than the line is cut between characters, at least
/// one a line. Without non-space wrap the rest of the text then fills the
/// lines character by character, its spaces included (one at a break
/// left out). The lines as ranges of `line`, and the first that cut a
/// word without non-space wrap.
fn breaks(
    line: &[char],
    fits: &dyn Fn(&[char]) -> bool,
    non_space: bool,
) -> (Vec<(usize, usize)>, Option<usize>) {
    let n = line.len();
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut s = 0;
    let mut greedy = false;
    let mut cut = None;
    while s < n {
        if !out.is_empty() && !greedy {
            while s < n && line[s] == ' ' {
                s += 1;
            }
            if s >= n {
                break;
            }
        }
        let mut last_ok = None;
        let mut i = s;
        while i < n {
            let j = i + 1;
            let mut upto = &line[s..j];
            if !greedy {
                while let [rest @ .., ' '] = upto {
                    upto = rest;
                }
            }
            if j - 1 > s && !fits(upto) {
                break;
            }
            if line[i] == '-' || (line[i] == ' ' && (j >= n || line[j] != ' ')) {
                last_ok = Some(j);
            }
            i = j;
        }
        if i >= n {
            out.push((s, n));
            break;
        }
        if let Some(end) = last_ok.filter(|e| !greedy && *e > s) {
            out.push((s, end));
            s = end;
            continue;
        }
        if !non_space && !greedy {
            greedy = true;
            cut = cut.or(Some(out.len()));
        }
        out.push((s, i));
        s = i;
        if greedy && s < n && line[s] == ' ' {
            s += 1;
        }
    }
    if out.is_empty() {
        out.push((0, 0));
    }
    (out, cut)
}

/// The lines the text breaks into at `max` units (none: its own lines),
/// as `layout` breaks them (decision T282), each without its trailing
/// spaces: what the renderer draws.
pub fn wrapped_lines(vm: &Vm, id: WidgetId, text: &str, max: Option<f64>) -> Vec<String> {
    let font = font_of(vm, id);
    let pixel = pixel(vm, id);
    let px = pixel_size(&font, pixel);
    let props = engine(vm).world.widgets[id.0].props;
    let non_space = crate::widgets::stored_ref(vm, props, "NonSpaceWrap")
        .and_then(|v| v.first().copied())
        .is_some_and(|v| v.truthy());
    let max = max.filter(|m| *m > 0.0 && font.size > 0.0);
    let units = |p: f64| f64::from(p as f32 * pixel);
    let mut out = Vec::new();
    for line in visible(text).split('\n') {
        let Some(m) = max else {
            out.push(line.to_string());
            continue;
        };
        let chars: Vec<char> = line.chars().collect();
        let fits = |s: &[char]| units(advance_px(&font, px, s)) <= m + 1e-3;
        let (ranges, _) = breaks(&chars, &fits, non_space);
        for (start, end) in ranges {
            let mut t = &chars[start..end];
            while let [rest @ .., ' '] = t {
                t = rest;
            }
            out.push(t.iter().collect());
        }
    }
    out
}

/// What the client makes of a text at a width (decision T282).
pub struct Layout {
    /// `GetNumLines()`: every line the text breaks into at the width, also
    /// with word wrap off or more lines than it may show.
    pub lines: usize,
    /// The lines it draws (its height): one per line of its own without
    /// word wrap; at most the maximum lines; with a word cut without
    /// non-space wrap and a maximum, to the line that cut it.
    pub shown: usize,
    /// `IsTruncated()`: lines it doesn't show, or a word cut without
    /// non-space wrap.
    pub truncated: bool,
    /// `GetStringWidth()`: the whole text's widest line, or, cut short by
    /// a maximum, the text it shows to the cut and an ellipsis.
    pub width: f64,
}

/// The text laid out in the region at `max` units (none: unbounded) as
/// the client does it (decision T282): `wraps` its word wrap, `max_lines`
/// its maximum (0: none). The ellipsis is one glyph three periods wide
/// that fits by a pixel less (recorded in FRIZQT 12 and ARIALN 14;
/// unverified at other sizes). The client breaks some narrow lines a line
/// sooner than whole-pixel advances say (its advances are finer).
pub fn layout(
    vm: &Vm,
    id: WidgetId,
    text: &str,
    max: Option<f64>,
    wraps: bool,
    max_lines: usize,
) -> Layout {
    let font = font_of(vm, id);
    let pixel = pixel(vm, id);
    let px = pixel_size(&font, pixel);
    let props = engine(vm).world.widgets[id.0].props;
    let non_space = crate::widgets::stored_ref(vm, props, "NonSpaceWrap")
        .and_then(|v| v.first().copied())
        .is_some_and(|v| v.truthy());
    let vis = visible(text);
    let full = width(vm, id, text);
    let hard: Vec<Vec<char>> = vis.split('\n').map(|l| l.chars().collect()).collect();
    let max = max.filter(|m| *m > 0.0 && font.size > 0.0);
    let units = |p: f64| f64::from(p as f32 * pixel);
    let font_ref = &font;
    let fits = |m: f64| move |s: &[char]| units(advance_px(font_ref, px, s)) <= m + 1e-3;
    // every line: its own line's index, its range
    let mut all: Vec<(usize, usize, usize)> = Vec::new();
    let mut cut = None;
    for (h, line) in hard.iter().enumerate() {
        match max {
            Some(m) => {
                let (ranges, c) = breaks(line, &fits(m), non_space);
                if cut.is_none() {
                    cut = c.map(|c| all.len() + c);
                }
                all.extend(ranges.into_iter().map(|(s, e)| (h, s, e)));
            }
            None => all.push((h, 0, line.len())),
        }
    }
    let lines = all.len();
    let whole = |shown: usize, truncated: bool| Layout {
        lines,
        shown,
        truncated,
        width: full,
    };
    if !wraps {
        return whole(hard.len(), lines > hard.len());
    }
    let shown = match (cut, max_lines) {
        (Some(_), 0) => return whole(lines, true),
        (Some(c), m) => (c + 1).min(m),
        (None, m) if m > 0 && lines > m => m,
        _ => return whole(lines, false),
    };
    // The shown text to the cut, and the ellipsis.
    let (row, scale) = row(&font, px);
    let dot = row.advances[('.' as u32 - 32) as usize] * scale;
    let (ellipsis_fit, ellipsis_width) = (3.0 * dot - 1.0, 3.0 * dot);
    let (h, s, e) = all[shown - 1];
    let line = &hard[h];
    let mut k = e;
    let trimmed = |k: usize| {
        let mut t = &line[s..k];
        while let [rest @ .., ' '] = t {
            t = rest;
        }
        t
    };
    while k > s
        && max.is_some_and(|m| units(advance_px(&font, px, trimmed(k)) + ellipsis_fit) > m + 1e-3)
    {
        k -= 1;
    }
    let mut head = &line[..k];
    while let [rest @ .., ' '] = head {
        head = rest;
    }
    let last = if head.is_empty() {
        units(ellipsis_width)
    } else {
        units(advance_px(&font, px, head) + ellipsis_width)
    };
    let before = hard[..h]
        .iter()
        .map(|l| units(line_px(&font, px, &l.iter().collect::<String>())))
        .fold(0.0, f64::max);
    Layout {
        lines,
        shown,
        truncated: true,
        width: before.max(last),
    }
}

/// The text laid out in the region at `max` units with its own word wrap
/// and maximum lines (`SetWordWrap`, `SetMaxLines`, a font object's).
pub fn layout_in(vm: &Vm, id: WidgetId, text: &str, max: Option<f64>) -> Layout {
    let props = engine(vm).world.widgets[id.0].props;
    let wraps = crate::widgets::stored_ref(vm, props, "WordWrap")
        .and_then(|v| v.first().copied())
        .is_none_or(|v| v.truthy());
    let max_lines =
        match crate::widgets::stored_ref(vm, props, "MaxLines").and_then(|v| v.first().copied()) {
            Some(Value::Num(n)) if n > 0.0 => n as usize,
            _ => 0,
        };
    layout(vm, id, text, max, wraps, max_lines)
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
        x += match inline_height(c) {
            Some(h) => inline_advance(&font, px, h),
            None => row.advances[index(c)] * scale,
        };
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
