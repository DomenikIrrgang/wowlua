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

fn json_string(s: &str) -> String {
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
        let mut sorted = self.segments.clone();
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
            json_string(&self.file),
            list(&self.sources),
            list(&self.names),
            json_string(&text)
        )
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

/// A Source Map v3 document for one output file with one source; `names`
/// gives the original names of renamed identifiers by source position.
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
    map.to_json()
}

/// A Source Map v3 document for one output file with one source.
pub fn source_map(mappings: &[Mapping], file: &str, source: &str) -> String {
    source_map_named(mappings, file, source, &std::collections::HashMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Pos;

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
