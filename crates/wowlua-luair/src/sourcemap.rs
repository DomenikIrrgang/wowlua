//! Source Map v3 output (spec codegen §14.6).

use crate::emit::Mapping;

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn vlq(out: &mut String, value: i64) {
    let mut v = if value < 0 {
        ((-value) << 1) | 1
    } else {
        value << 1
    } as u64;
    loop {
        let mut digit = (v & 31) as usize;
        v >>= 5;
        if v > 0 {
            digit |= 32;
        }
        out.push(BASE64[digit] as char);
        if v == 0 {
            break;
        }
    }
}

/// Appends `s` as a JSON string.
fn json_into(out: &mut String, s: &str) {
    out.push('"');
    if !s.bytes().any(|b| b == b'"' || b == b'\\' || b < 0x20) {
        out.push_str(s);
    } else {
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
    }
    out.push('"');
}

/// The `mappings` text of segments in output order.
struct Encoder {
    text: String,
    line: u32,
    first_in_line: bool,
    prev_out_col: i64,
    prev_source: i64,
    prev_line: i64,
    prev_col: i64,
    prev_name: i64,
}

impl Encoder {
    fn new() -> Encoder {
        Encoder {
            text: String::new(),
            line: 1,
            first_in_line: true,
            prev_out_col: 0,
            prev_source: 0,
            prev_line: 0,
            prev_col: 0,
            prev_name: 0,
        }
    }

    fn push(&mut self, m: &Segment) {
        while self.line < m.out_line {
            self.text.push(';');
            self.line += 1;
            self.first_in_line = true;
            self.prev_out_col = 0;
        }
        if !self.first_in_line {
            self.text.push(',');
        }
        self.first_in_line = false;
        let src_line = i64::from(m.src_line) - 1;
        vlq(&mut self.text, i64::from(m.out_col) - self.prev_out_col);
        vlq(&mut self.text, i64::from(m.source) - self.prev_source);
        vlq(&mut self.text, src_line - self.prev_line);
        vlq(&mut self.text, i64::from(m.src_col) - self.prev_col);
        if let Some(n) = m.name {
            vlq(&mut self.text, i64::from(n) - self.prev_name);
            self.prev_name = i64::from(n);
        }
        self.prev_out_col = i64::from(m.out_col);
        self.prev_source = i64::from(m.source);
        self.prev_line = src_line;
        self.prev_col = i64::from(m.src_col);
    }

    fn finish(mut self) -> String {
        self.text.shrink_to_fit();
        self.text
    }
}

/// One mapping: an output position (1-based line, 0-based column), its
/// source (an index into `sources`), the source position, and the original
/// name of a renamed identifier (an index into `names`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub out_line: u32,
    pub out_col: u32,
    pub source: u32,
    pub src_line: u32,
    pub src_col: u32,
    pub name: Option<u32>,
}

/// A Source Map v3 document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceMap {
    pub file: String,
    pub sources: Vec<String>,
    pub names: Vec<String>,
    pub segments: Vec<Segment>,
}

impl SourceMap {
    pub fn to_json(&self) -> String {
        self.encode().to_json()
    }

    /// The map with its segments in output order (a stable sort of them,
    /// when they are not in that order).
    pub fn encode(&self) -> EncodedMap {
        let key = |m: &Segment| (m.out_line, m.out_col);
        let mut encoder = Encoder::new();
        if self.segments.is_sorted_by_key(key) {
            self.segments.iter().for_each(|m| encoder.push(m));
        } else {
            let mut sorted = self.segments.clone();
            sorted.sort_by_key(key);
            sorted.iter().for_each(|m| encoder.push(m));
        }
        EncodedMap {
            file: self.file.clone(),
            sources: self.sources.clone(),
            names: self.names.clone(),
            mappings: encoder.finish(),
        }
    }

    /// Reads a Source Map v3 document (as [`SourceMap::to_json`] writes).
    pub fn parse(json: &str) -> Option<SourceMap> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        let strings = |key: &str| -> Vec<String> {
            value[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        };
        let mappings = value["mappings"].as_str()?;
        let mut segments = Vec::new();
        let (mut source, mut src_line, mut src_col, mut name) = (0i64, 0i64, 0i64, 0i64);
        for (i, line) in mappings.split(';').enumerate() {
            let mut out_col = 0i64;
            for seg in line.split(',').filter(|s| !s.is_empty()) {
                let fields = unvlq(seg)?;
                if fields.len() < 4 {
                    continue;
                }
                out_col += fields[0];
                source += fields[1];
                src_line += fields[2];
                src_col += fields[3];
                let n = fields.get(4).map(|d| {
                    name += d;
                    name as u32
                });
                segments.push(Segment {
                    out_line: i as u32 + 1,
                    out_col: out_col as u32,
                    source: source as u32,
                    src_line: src_line as u32 + 1,
                    src_col: src_col as u32,
                    name: n,
                });
            }
        }
        Some(SourceMap {
            file: value["file"].as_str().unwrap_or_default().to_string(),
            sources: strings("sources"),
            names: strings("names"),
            segments,
        })
    }
}

fn unvlq(text: &str) -> Option<Vec<i64>> {
    let mut out = Vec::new();
    let mut value: u64 = 0;
    let mut shift = 0;
    for c in text.bytes() {
        let digit = BASE64.iter().position(|&b| b == c)? as u64;
        value |= (digit & 31) << shift;
        if digit & 32 != 0 {
            shift += 5;
            continue;
        }
        let v = (value >> 1) as i64;
        out.push(if value & 1 == 1 { -v } else { v });
        value = 0;
        shift = 0;
    }
    Some(out)
}

/// A Source Map v3 document with its mappings encoded: what it takes in
/// memory until it is written ([`EncodedMap::to_json`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EncodedMap {
    pub file: String,
    pub sources: Vec<String>,
    pub names: Vec<String>,
    /// The VLQ mappings, as [`SourceMap::to_json`] writes them.
    mappings: String,
}

/// Base64 digit values by byte; `NOT_BASE64` for other bytes.
const DECODE: [u8; 256] = {
    let mut table = [NOT_BASE64; 256];
    let mut i = 0;
    while i < 64 {
        table[BASE64[i] as usize] = i as u8;
        i += 1;
    }
    table
};
const NOT_BASE64: u8 = 255;

impl EncodedMap {
    /// The JSON document, the same bytes as [`SourceMap::to_json`] of the
    /// same map.
    pub fn to_json(&self) -> String {
        let strings = |items: &[String]| items.iter().map(|s| s.len() + 2).sum::<usize>();
        let mut out = String::with_capacity(
            64 + self.file.len()
                + strings(&self.sources)
                + strings(&self.names)
                + self.sources.len()
                + self.names.len()
                + self.mappings.len(),
        );
        out.push_str("{\"version\":3,\"file\":");
        json_into(&mut out, &self.file);
        let list = |out: &mut String, items: &[String]| {
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json_into(out, item);
            }
        };
        out.push_str(",\"sources\":[");
        list(&mut out, &self.sources);
        out.push_str("],\"names\":[");
        list(&mut out, &self.names);
        out.push_str("],\"mappings\":");
        json_into(&mut out, &self.mappings);
        out.push_str("}\n");
        out
    }

    /// Whether it decodes: `self.decode().is_some()`.
    pub fn parses(&self) -> bool {
        self.mappings
            .bytes()
            .all(|b| b == b';' || b == b',' || DECODE[b as usize] != NOT_BASE64)
    }

    /// The map's segments: `SourceMap::parse(&self.to_json())`.
    pub fn decode(&self) -> Option<SourceMap> {
        let mut segments = Vec::new();
        decode_mappings(&self.mappings, |s| segments.push(s))?;
        Some(SourceMap {
            file: self.file.clone(),
            sources: self.sources.clone(),
            names: self.names.clone(),
            segments,
        })
    }

    /// Calls `f` with each segment [`EncodedMap::decode`] gives, in order;
    /// nothing when the map does not decode.
    pub fn for_each_segment(&self, mut f: impl FnMut(Segment)) {
        if self.parses() {
            let _ = decode_mappings(&self.mappings, &mut f);
        }
    }
}

/// Decodes `mappings` as [`SourceMap::parse`] does, calling `f` with each
/// segment; `None` (after calls for earlier segments) when a byte is not
/// Base64. The arithmetic is `parse`'s, overflow behavior included.
fn decode_mappings(mappings: &str, mut f: impl FnMut(Segment)) -> Option<()> {
    let (mut source, mut src_line, mut src_col, mut name) = (0i64, 0i64, 0i64, 0i64);
    for (i, line) in mappings.split(';').enumerate() {
        let mut out_col = 0i64;
        for seg in line.split(',').filter(|s| !s.is_empty()) {
            // The values of the segment (only the first five are used).
            let mut fields = [0i64; 5];
            let mut count = 0usize;
            let mut value: u64 = 0;
            let mut shift = 0;
            for c in seg.bytes() {
                let digit = DECODE[c as usize];
                if digit == NOT_BASE64 {
                    return None;
                }
                let digit = u64::from(digit);
                value |= (digit & 31) << shift;
                if digit & 32 != 0 {
                    shift += 5;
                    continue;
                }
                let v = (value >> 1) as i64;
                if count < fields.len() {
                    fields[count] = if value & 1 == 1 { -v } else { v };
                }
                count += 1;
                value = 0;
                shift = 0;
            }
            if count < 4 {
                continue;
            }
            out_col += fields[0];
            source += fields[1];
            src_line += fields[2];
            src_col += fields[3];
            let n = (count > 4).then(|| {
                name += fields[4];
                name as u32
            });
            f(Segment {
                out_line: i as u32 + 1,
                out_col: out_col as u32,
                source: source as u32,
                src_line: src_line as u32 + 1,
                src_col: src_col as u32,
                name: n,
            });
        }
    }
    Some(())
}

/// Encodes segments as they come, for callers whose segments are in output
/// order: the same text as [`SourceMap::encode`] of them. A segment before
/// the previous one stops it ([`SegmentEncoder::push`] returns `false`), and
/// the caller encodes them sorted instead.
pub struct SegmentEncoder {
    encoder: Encoder,
    last: (u32, u32),
    in_order: bool,
}

impl Default for SegmentEncoder {
    fn default() -> Self {
        SegmentEncoder {
            encoder: Encoder::new(),
            last: (0, 0),
            in_order: true,
        }
    }
}

impl SegmentEncoder {
    /// Adds the next segment; `false` once one came before its predecessor.
    pub fn push(&mut self, segment: &Segment) -> bool {
        let key = (segment.out_line, segment.out_col);
        if self.in_order && key >= self.last {
            self.last = key;
            self.encoder.push(segment);
        } else {
            self.in_order = false;
        }
        self.in_order
    }

    /// The map, or `None` when a segment came out of order.
    pub fn finish(
        self,
        file: String,
        sources: Vec<String>,
        names: Vec<String>,
    ) -> Option<EncodedMap> {
        self.in_order.then(|| EncodedMap {
            file,
            sources,
            names,
            mappings: self.encoder.finish(),
        })
    }
}

/// The map of one output file with one source, encoded; `names` gives the
/// original names of renamed identifiers by source position. Name indices
/// follow the order of `mappings`.
pub fn encode_named(
    mappings: &[Mapping],
    file: &str,
    source: &str,
    names: &std::collections::HashMap<crate::ir::Pos, String>,
) -> EncodedMap {
    let mut out_names = Vec::new();
    let mut index: std::collections::HashMap<&String, u32> = std::collections::HashMap::new();
    let mut name_of = |m: &Mapping| {
        names.get(&m.src).map(|n| {
            *index.entry(n).or_insert_with(|| {
                out_names.push(n.clone());
                out_names.len() as u32 - 1
            })
        })
    };
    let segment = |m: &Mapping, name: Option<u32>| Segment {
        out_line: m.out_line,
        out_col: m.out_col,
        source: 0,
        src_line: m.src.line,
        src_col: m.src.col,
        name,
    };
    // The emitter's positions only move forward.
    if !mappings.is_sorted_by_key(|m| (m.out_line, m.out_col)) {
        let segments = mappings.iter().map(|m| segment(m, name_of(m))).collect();
        return SourceMap {
            file: file.to_string(),
            sources: vec![source.to_string()],
            names: out_names,
            segments,
        }
        .encode();
    }
    let mut encoder = Encoder::new();
    for m in mappings {
        encoder.push(&segment(m, name_of(m)));
    }
    EncodedMap {
        file: file.to_string(),
        sources: vec![source.to_string()],
        names: out_names,
        mappings: encoder.finish(),
    }
}

/// A Source Map v3 document for one output file with one source; `names`
/// gives the original names of renamed identifiers by source position.
pub fn source_map_named(
    mappings: &[Mapping],
    file: &str,
    source: &str,
    names: &std::collections::HashMap<crate::ir::Pos, String>,
) -> String {
    encode_named(mappings, file, source, names).to_json()
}

/// A Source Map v3 document for one output file with one source.
pub fn source_map(mappings: &[Mapping], file: &str, source: &str) -> String {
    source_map_named(mappings, file, source, &std::collections::HashMap::new())
}

/// The encoding before [`EncodedMap`], kept to compare against.
#[cfg(test)]
mod reference {
    use super::{Segment, SourceMap, vlq};
    use crate::emit::Mapping;

    pub fn json_string(s: &str) -> String {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    pub fn to_json(map: &SourceMap) -> String {
        let mut sorted = map.segments.clone();
        sorted.sort_by_key(|m| (m.out_line, m.out_col));
        let mut text = String::new();
        let mut line = 1;
        let (mut prev_source, mut prev_line, mut prev_col, mut prev_name) =
            (0i64, 0i64, 0i64, 0i64);
        let mut first_in_line = true;
        let mut prev_out_col: i64 = 0;
        for m in sorted {
            while line < m.out_line {
                text.push(';');
                line += 1;
                first_in_line = true;
                prev_out_col = 0;
            }
            if !first_in_line {
                text.push(',');
            }
            first_in_line = false;
            let src_line = i64::from(m.src_line) - 1;
            vlq(&mut text, i64::from(m.out_col) - prev_out_col);
            vlq(&mut text, i64::from(m.source) - prev_source);
            vlq(&mut text, src_line - prev_line);
            vlq(&mut text, i64::from(m.src_col) - prev_col);
            if let Some(n) = m.name {
                vlq(&mut text, i64::from(n) - prev_name);
                prev_name = i64::from(n);
            }
            prev_out_col = i64::from(m.out_col);
            prev_source = i64::from(m.source);
            prev_line = src_line;
            prev_col = i64::from(m.src_col);
        }
        let list = |items: &[String]| {
            items
                .iter()
                .map(|s| json_string(s))
                .collect::<Vec<_>>()
                .join(",")
        };
        format!(
            "{{\"version\":3,\"file\":{},\"sources\":[{}],\"names\":[{}],\"mappings\":{}}}\n",
            json_string(&map.file),
            list(&map.sources),
            list(&map.names),
            json_string(&text)
        )
    }

    pub fn source_map_named(
        mappings: &[Mapping],
        file: &str,
        source: &str,
        names: &std::collections::HashMap<crate::ir::Pos, String>,
    ) -> String {
        let mut map = SourceMap {
            file: file.to_string(),
            sources: vec![source.to_string()],
            ..SourceMap::default()
        };
        let mut index: std::collections::HashMap<&String, u32> = std::collections::HashMap::new();
        for m in mappings {
            let name = names.get(&m.src).map(|n| {
                *index.entry(n).or_insert_with(|| {
                    map.names.push(n.clone());
                    map.names.len() as u32 - 1
                })
            });
            map.segments.push(Segment {
                out_line: m.out_line,
                out_col: m.out_col,
                source: 0,
                src_line: m.src.line,
                src_col: m.src.col,
                name,
            });
        }
        to_json(&map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Pos;
    use std::collections::HashMap;

    /// A small deterministic generator (xorshift), so failures reproduce.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn mappings(rng: &mut Rng, kind: u64) -> Vec<Mapping> {
        let len = rng.below(60) as usize;
        let mut out: Vec<Mapping> = (0..len)
            .map(|_| {
                let big = rng.below(10) == 0;
                Mapping {
                    out_line: 1 + rng.below(if big { 5000 } else { 12 }) as u32,
                    out_col: rng.below(if big { u64::from(u32::MAX) } else { 80 }) as u32,
                    src: Pos::new(
                        rng.below(if big { u64::from(u32::MAX) } else { 40 }) as u32,
                        rng.below(if big { u64::from(u32::MAX) } else { 80 }) as u32,
                    ),
                }
            })
            .collect();
        match kind {
            // In output order, as the emitter makes them.
            0 => out.sort_by_key(|m| (m.out_line, m.out_col)),
            // In output order with runs of equal positions.
            1 => {
                out.sort_by_key(|m| (m.out_line, m.out_col));
                for i in 1..out.len() {
                    if rng.below(3) == 0 {
                        out[i].out_line = out[i - 1].out_line;
                        out[i].out_col = out[i - 1].out_col;
                    }
                }
            }
            _ => {}
        }
        out
    }

    fn text(rng: &mut Rng) -> String {
        const PARTS: &[&str] = &[
            "a",
            "src/x.lua",
            "\"",
            "\\",
            "\n",
            "\t",
            "\u{1}",
            "é",
            "名",
            "",
        ];
        (0..rng.below(5))
            .map(|_| PARTS[rng.below(PARTS.len() as u64) as usize])
            .collect()
    }

    #[test]
    fn encoding_matches_the_old_one() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for round in 0..3000 {
            let ms = mappings(&mut rng, round % 3);
            let mut names = HashMap::new();
            for m in &ms {
                if rng.below(3) == 0 {
                    // The same name at several positions too.
                    names.insert(m.src, format!("n{}", rng.below(4)));
                }
            }
            let (file, source) = (text(&mut rng), text(&mut rng));
            let new = encode_named(&ms, &file, &source, &names);
            assert_eq!(
                new.to_json(),
                reference::source_map_named(&ms, &file, &source, &names),
                "round {round}"
            );
            let map = SourceMap {
                file: text(&mut rng),
                sources: (0..rng.below(4)).map(|_| text(&mut rng)).collect(),
                names: (0..rng.below(4)).map(|_| text(&mut rng)).collect(),
                segments: ms
                    .iter()
                    .map(|m| Segment {
                        out_line: m.out_line,
                        out_col: m.out_col,
                        source: rng.below(4) as u32,
                        src_line: m.src.line,
                        src_col: m.src.col,
                        name: (rng.below(2) == 0).then(|| rng.below(4) as u32),
                    })
                    .collect(),
            };
            assert_eq!(
                map.encode().to_json(),
                reference::to_json(&map),
                "round {round}"
            );
            assert_eq!(map.to_json(), reference::to_json(&map), "round {round}");
        }
    }

    #[test]
    fn json_strings_match_the_old_ones() {
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        for _ in 0..2000 {
            let s = text(&mut rng);
            let mut out = String::new();
            json_into(&mut out, &s);
            assert_eq!(out, reference::json_string(&s));
        }
    }

    /// `decode` and `parses` against `SourceMap::parse` (unchanged), on
    /// valid, invalid and odd mappings: both panic (debug overflow) or both
    /// give the same.
    #[test]
    fn decoding_matches_parse() {
        const BYTES: &[u8] = b"AAAACgBgC+/9zZ;;,,,,; !\"\\";
        let mut rng = Rng(0x0DDB_1A5E_5BAD_5EED);
        for round in 0..20_000 {
            let len = rng.below(40) as usize;
            let mut mappings: Vec<u8> = (0..len)
                .map(|_| BYTES[rng.below(BYTES.len() as u64) as usize])
                .collect();
            if round % 2 == 0 {
                mappings.retain(|b| b.is_ascii_alphanumeric() || b"+/;,".contains(b));
            }
            let mappings = String::from_utf8(mappings).unwrap();
            let map = EncodedMap {
                file: "f".into(),
                sources: vec!["s".into()],
                names: vec!["n".into()],
                mappings,
            };
            let old = std::panic::catch_unwind(|| SourceMap::parse(&map.to_json()));
            let new = std::panic::catch_unwind(|| map.decode());
            match (old, new) {
                (Ok(old), Ok(new)) => {
                    assert_eq!(new, old, "{:?}", map.mappings);
                    assert_eq!(map.parses(), old.is_some(), "{:?}", map.mappings);
                    let mut streamed = Vec::new();
                    map.for_each_segment(|s| streamed.push(s));
                    assert_eq!(streamed, old.map(|m| m.segments).unwrap_or_default());
                }
                (Err(_), Err(_)) => {}
                (old, new) => panic!("{:?}: {old:?} vs {new:?}", map.mappings),
            }
        }
    }

    #[test]
    fn encodes_vlq_segments() {
        let mut s = String::new();
        for v in [0, 1, -1, 15, 16, 1000] {
            vlq(&mut s, v);
            s.push(' ');
        }
        assert_eq!(s, "A C D e gB w+B ");
        let map = source_map(
            &[
                Mapping {
                    out_line: 1,
                    out_col: 0,
                    src: Pos::new(1, 0),
                },
                Mapping {
                    out_line: 1,
                    out_col: 6,
                    src: Pos::new(1, 6),
                },
                Mapping {
                    out_line: 3,
                    out_col: 0,
                    src: Pos::new(4, 2),
                },
            ],
            "a.lua",
            "a.wowlua",
        );
        assert!(map.contains("\"mappings\":\"AAAA,MAAM;;AAGJ\""), "{map}");
        let parsed = SourceMap::parse(&map).unwrap();
        assert_eq!(parsed.segments.len(), 3);
        assert_eq!(parsed.segments[2].src_line, 4);
        assert_eq!(SourceMap::parse(&parsed.to_json()).unwrap(), parsed);
    }
}
