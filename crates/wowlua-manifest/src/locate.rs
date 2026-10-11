//! Finds the source location of a JSON value by its path, so that semantic
//! manifest errors can point at the offending key or value.
//!
//! The input is assumed to be valid JSON (it has already been parsed once).

use std::fmt;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathSeg {
    Key(String),
    Index(usize),
}

impl From<&str> for PathSeg {
    fn from(key: &str) -> Self {
        PathSeg::Key(key.to_string())
    }
}

impl From<usize> for PathSeg {
    fn from(index: usize) -> Self {
        PathSeg::Index(index)
    }
}

/// A path into a JSON document, e.g. `addon.saved-variables[2]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsonPath(pub Vec<PathSeg>);

impl JsonPath {
    pub fn root() -> Self {
        JsonPath(Vec::new())
    }

    pub fn key(mut self, key: &str) -> Self {
        self.0.push(PathSeg::Key(key.to_string()));
        self
    }

    pub fn index(mut self, index: usize) -> Self {
        self.0.push(PathSeg::Index(index));
        self
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The path without its last segment.
    pub fn parent(&self) -> JsonPath {
        let mut segs = self.0.clone();
        segs.pop();
        JsonPath(segs)
    }
}

impl fmt::Display for JsonPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str("(root)");
        }
        for (i, seg) in self.0.iter().enumerate() {
            match seg {
                PathSeg::Key(key) if i == 0 => write!(f, "{key}")?,
                PathSeg::Key(key) => write!(f, ".{key}")?,
                PathSeg::Index(index) => write!(f, "[{index}]")?,
            }
        }
        Ok(())
    }
}

/// The location of a value: the span of its object key (if it has one) and
/// the span of the value itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Located {
    pub key: Option<Range<usize>>,
    pub value: Range<usize>,
}

impl Located {
    /// The key span when present, otherwise the value span.
    pub fn key_or_value(&self) -> Range<usize> {
        self.key.clone().unwrap_or_else(|| self.value.clone())
    }
}

/// Locates `path` in `text`. Returns `None` if the path does not exist.
pub fn locate(text: &str, path: &JsonPath) -> Option<Located> {
    let mut scanner = Scanner {
        bytes: text.as_bytes(),
        pos: 0,
    };
    scanner.skip_ws();
    scanner.find(&path.0, None)
}

struct Scanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Scanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    /// Walks the value at the current position. If `path` is empty, returns
    /// this value's location; otherwise descends.
    fn find(&mut self, path: &[PathSeg], key: Option<Range<usize>>) -> Option<Located> {
        let start = self.pos;
        match (path.first(), self.peek()?) {
            (None, _) => {
                self.skip_value()?;
                Some(Located {
                    key,
                    value: start..self.pos,
                })
            }
            (Some(PathSeg::Key(wanted)), b'{') => {
                self.pos += 1;
                loop {
                    self.skip_ws();
                    if self.peek()? == b'}' {
                        return None;
                    }
                    let key_start = self.pos;
                    let name = self.string()?;
                    let key_range = key_start..self.pos;
                    self.skip_ws();
                    self.expect(b':')?;
                    self.skip_ws();
                    if name == *wanted {
                        return self.find(&path[1..], Some(key_range));
                    }
                    self.skip_value()?;
                    self.skip_ws();
                    if self.peek()? == b',' {
                        self.pos += 1;
                    }
                }
            }
            (Some(PathSeg::Index(wanted)), b'[') => {
                self.pos += 1;
                let mut index = 0;
                loop {
                    self.skip_ws();
                    if self.peek()? == b']' {
                        return None;
                    }
                    if index == *wanted {
                        return self.find(&path[1..], None);
                    }
                    self.skip_value()?;
                    self.skip_ws();
                    if self.peek()? == b',' {
                        self.pos += 1;
                    }
                    index += 1;
                }
            }
            _ => None,
        }
    }

    fn expect(&mut self, byte: u8) -> Option<()> {
        (self.peek()? == byte).then(|| self.pos += 1)
    }

    fn string(&mut self) -> Option<String> {
        let start = self.pos;
        self.expect(b'"')?;
        while self.peek()? != b'"' {
            if self.peek()? == b'\\' {
                self.pos += 1;
            }
            self.pos += 1;
        }
        self.pos += 1;
        serde_json::from_slice(&self.bytes[start..self.pos]).ok()
    }

    fn skip_value(&mut self) -> Option<()> {
        match self.peek()? {
            b'"' => self.string().map(drop),
            open @ (b'{' | b'[') => {
                let close = if open == b'{' { b'}' } else { b']' };
                self.pos += 1;
                loop {
                    self.skip_ws();
                    if self.peek()? == close {
                        self.pos += 1;
                        return Some(());
                    }
                    if open == b'{' {
                        self.string()?;
                        self.skip_ws();
                        self.expect(b':')?;
                        self.skip_ws();
                    }
                    self.skip_value()?;
                    self.skip_ws();
                    if self.peek()? == b',' {
                        self.pos += 1;
                    }
                }
            }
            _ => {
                while let Some(b) = self.peek() {
                    if matches!(b, b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                        break;
                    }
                    self.pos += 1;
                }
                Some(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"{
  "package": { "name": "a", "version": "1.0.0" },
  "addon": { "saved-variables": ["A", "B\"x", "C"], "title": { "nested": true } }
}"#;

    fn slice(range: Range<usize>) -> &'static str {
        &TEXT[range]
    }

    #[test]
    fn locates_keys_values_and_indexes() {
        let path = JsonPath::root().key("package").key("version");
        let found = locate(TEXT, &path).unwrap();
        assert_eq!(slice(found.key.unwrap()), "\"version\"");
        assert_eq!(slice(found.value), "\"1.0.0\"");

        let path = JsonPath::root()
            .key("addon")
            .key("saved-variables")
            .index(2);
        assert_eq!(slice(locate(TEXT, &path).unwrap().value), "\"C\"");

        let path = JsonPath::root().key("addon").key("title");
        assert_eq!(
            slice(locate(TEXT, &path).unwrap().value),
            "{ \"nested\": true }"
        );
    }

    #[test]
    fn missing_paths_return_none() {
        assert!(locate(TEXT, &JsonPath::root().key("nope")).is_none());
        let path = JsonPath::root()
            .key("addon")
            .key("saved-variables")
            .index(9);
        assert!(locate(TEXT, &path).is_none());
    }

    #[test]
    fn displays_paths() {
        let path = JsonPath::root()
            .key("addon")
            .key("saved-variables")
            .index(1);
        assert_eq!(path.to_string(), "addon.saved-variables[1]");
    }
}
