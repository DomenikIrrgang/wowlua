//! Positions as the protocol counts them (a line and UTF-16 code units in
//! it) and file URIs.

use serde_json::{Value as Json, json};
use std::path::{Path, PathBuf};

/// The protocol's position of byte `offset` in `text`.
pub fn position(text: &str, offset: usize) -> Json {
    let offset = offset.min(text.len());
    let before = &text[..floor_char(text, offset)];
    let line = before.matches('\n').count();
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    let character: usize = before[start..].chars().map(char::len_utf16).sum();
    json!({ "line": line, "character": character })
}

/// A byte range as the protocol's range.
pub fn range(text: &str, start: usize, end: usize) -> Json {
    json!({ "start": position(text, start), "end": position(text, end) })
}

/// A text's line starts, for many positions in a large text: each
/// position costs a search and its line's length, not the text's.
pub struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    pub fn new(text: &'a str) -> Lines<'a> {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Lines { text, starts }
    }

    /// The line and UTF-16 column of byte `offset`.
    pub fn line_column(&self, offset: usize) -> (u32, u32) {
        let offset = floor_char(self.text, offset.min(self.text.len()));
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let start = self.starts[line];
        let column: usize = self.text[start..offset].chars().map(char::len_utf16).sum();
        (line as u32, column as u32)
    }

    /// [`position`] of byte `offset`.
    pub fn position(&self, offset: usize) -> Json {
        let (line, character) = self.line_column(offset);
        json!({ "line": line, "character": character })
    }

    /// [`range`] of bytes `start..end`.
    pub fn range(&self, start: usize, end: usize) -> Json {
        json!({ "start": self.position(start), "end": self.position(end) })
    }
}

/// The byte offset of the protocol's `{ line, character }` in `text`.
pub fn offset(text: &str, pos: &Json) -> usize {
    let line = pos["line"].as_u64().unwrap_or(0) as usize;
    let character = pos["character"].as_u64().unwrap_or(0) as usize;
    let mut start = 0;
    for _ in 0..line {
        match text[start..].find('\n') {
            Some(i) => start += i + 1,
            None => return text.len(),
        }
    }
    let mut units = 0;
    for (i, c) in text[start..].char_indices() {
        if units >= character || c == '\n' {
            return start + i;
        }
        units += c.len_utf16();
    }
    text.len()
}

fn floor_char(text: &str, mut i: usize) -> usize {
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// `path` canonical (symbolic links resolved), without Windows' verbatim
/// prefix (`\\?\C:\x` is `C:\x`), which editors' URIs never have; `path`
/// as it is when it does not exist.
pub fn canonical(path: &Path) -> PathBuf {
    let found = wowlua_driver::host::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = found.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => found,
    }
}

/// A `file:` URI's path.
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::new();
    let raw = rest.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%'
            && i + 2 < raw.len()
            && let Ok(b) = u8::from_str_radix(&rest[i + 1..i + 3], 16)
        {
            bytes.push(b);
            i += 3;
            continue;
        }
        bytes.push(raw[i]);
        i += 1;
    }
    let text = String::from_utf8(bytes).ok()?;
    // `file:///C:/x` on Windows
    let text = match text.strip_prefix('/') {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_string(),
        _ => text,
    };
    Some(PathBuf::from(text))
}

/// A path's `file:` URI.
pub fn uri_of(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_utf16() {
        let text = "a\nßx😀y\n";
        assert_eq!(position(text, 2), json!({ "line": 1, "character": 0 }));
        // ß is 2 bytes, 1 unit; 😀 4 bytes, 2 units
        let y = text.find('y').unwrap();
        assert_eq!(position(text, y), json!({ "line": 1, "character": 4 }));
        assert_eq!(offset(text, &json!({ "line": 1, "character": 4 })), y);
    }

    #[test]
    fn lines_give_the_same_positions() {
        let text = "a\nßx😀y\n\nlast";
        let lines = Lines::new(text);
        for i in 0..=text.len() + 2 {
            assert_eq!(lines.position(i), position(text, i), "offset {i}");
        }
    }

    #[test]
    fn uris_and_paths() {
        let p = PathBuf::from("/tmp/my addon/a.wowlua");
        let uri = uri_of(&p);
        assert_eq!(uri, "file:///tmp/my%20addon/a.wowlua");
        assert_eq!(path_of(&uri).unwrap(), p);
        assert_eq!(
            path_of("file:///C:/x/a.wowlua").unwrap(),
            PathBuf::from("C:/x/a.wowlua")
        );
    }
}
