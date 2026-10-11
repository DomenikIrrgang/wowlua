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

/// A Source Map v3 document for one output file with one source.
pub fn source_map(mappings: &[Mapping], file: &str, source: &str) -> String {
    let mut sorted: Vec<Mapping> = mappings.to_vec();
    sorted.sort_by_key(|m| (m.out_line, m.out_col));
    let mut text = String::new();
    let mut line = 1;
    let mut prev_src_line: i64 = 0;
    let mut prev_src_col: i64 = 0;
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
        let src_line = i64::from(m.src.line) - 1;
        let src_col = i64::from(m.src.col);
        vlq(&mut text, i64::from(m.out_col) - prev_out_col);
        vlq(&mut text, 0);
        vlq(&mut text, src_line - prev_src_line);
        vlq(&mut text, src_col - prev_src_col);
        prev_out_col = i64::from(m.out_col);
        prev_src_line = src_line;
        prev_src_col = src_col;
    }
    format!(
        "{{\"version\":3,\"file\":{},\"sources\":[{}],\"names\":[],\"mappings\":{}}}\n",
        json_string(file),
        json_string(source),
        json_string(&text)
    )
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
    }
}
