//! In-place edits of `wowlua.json` (spec CLI §12.3): `add`, `remove` and
//! `update` change only the affected entries. Key order, indentation and
//! line endings of the rest of the file are preserved.

use serde_json::Value;

#[derive(Debug)]
enum Kind {
    Object(Vec<Member>),
    Array(Vec<Node>),
    Scalar,
}

#[derive(Debug)]
struct Node {
    start: usize,
    end: usize,
    kind: Kind,
}

#[derive(Debug)]
struct Member {
    key: String,
    key_start: usize,
    value: Node,
}

struct Parser<'a> {
    text: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.pos < self.text.len() && self.text[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn string(&mut self) -> Result<String, String> {
        let start = self.pos;
        self.pos += 1;
        while self.pos < self.text.len() {
            match self.text[self.pos] {
                b'\\' => self.pos += 2,
                b'"' => {
                    self.pos += 1;
                    let raw = std::str::from_utf8(&self.text[start..self.pos])
                        .map_err(|e| e.to_string())?;
                    return serde_json::from_str(raw).map_err(|e| e.to_string());
                }
                _ => self.pos += 1,
            }
        }
        Err("unterminated string".into())
    }

    fn value(&mut self) -> Result<Node, String> {
        self.ws();
        let start = self.pos;
        match self.text.get(self.pos) {
            Some(b'{') => {
                self.pos += 1;
                let mut members = Vec::new();
                loop {
                    self.ws();
                    match self.text.get(self.pos) {
                        Some(b'}') => {
                            self.pos += 1;
                            break;
                        }
                        Some(b',') => self.pos += 1,
                        Some(b'"') => {
                            let key_start = self.pos;
                            let key = self.string()?;
                            self.ws();
                            if self.text.get(self.pos) != Some(&b':') {
                                return Err(format!("expected `:` at byte {}", self.pos));
                            }
                            self.pos += 1;
                            let value = self.value()?;
                            members.push(Member {
                                key,
                                key_start,
                                value,
                            });
                        }
                        _ => return Err(format!("unexpected character at byte {}", self.pos)),
                    }
                }
                Ok(Node {
                    start,
                    end: self.pos,
                    kind: Kind::Object(members),
                })
            }
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    match self.text.get(self.pos) {
                        Some(b']') => {
                            self.pos += 1;
                            break;
                        }
                        Some(b',') => self.pos += 1,
                        Some(_) => items.push(self.value()?),
                        None => return Err("unterminated array".into()),
                    }
                }
                Ok(Node {
                    start,
                    end: self.pos,
                    kind: Kind::Array(items),
                })
            }
            Some(b'"') => {
                self.string()?;
                Ok(Node {
                    start,
                    end: self.pos,
                    kind: Kind::Scalar,
                })
            }
            Some(_) => {
                while self.pos < self.text.len()
                    && !matches!(self.text[self.pos], b',' | b'}' | b']')
                    && !self.text[self.pos].is_ascii_whitespace()
                {
                    self.pos += 1;
                }
                Ok(Node {
                    start,
                    end: self.pos,
                    kind: Kind::Scalar,
                })
            }
            None => Err("unexpected end of input".into()),
        }
    }
}

fn parse(text: &str) -> Result<Node, String> {
    let mut p = Parser {
        text: text.as_bytes(),
        pos: 0,
    };
    p.value()
}

/// The file's indentation unit (`"  "`, `"    "` or `"\t"`) and newline.
fn style(text: &str) -> (String, &'static str) {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let unit = text
        .lines()
        .find_map(|l| {
            let indent: String = l.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            (!indent.is_empty() && indent.len() < l.len()).then_some(indent)
        })
        .unwrap_or_else(|| "  ".into());
    (unit, newline)
}

/// The indentation of the line containing byte `pos`.
fn line_indent(text: &str, pos: usize) -> String {
    let line_start = text[..pos].rfind('\n').map_or(0, |i| i + 1);
    text[line_start..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// `value` pretty-printed with `unit`, continuation lines prefixed by `base`.
fn render(value: &Value, unit: &str, base: &str, newline: &str) -> String {
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(unit.as_bytes());
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    serde::Serialize::serialize(value, &mut ser).expect("JSON values serialize");
    let text = String::from_utf8(buf).expect("serde_json writes UTF-8");
    text.replace('\n', &format!("{newline}{base}"))
}

fn find<'a>(node: &'a Node, path: &[&str]) -> Option<&'a Node> {
    let mut current = node;
    for segment in path {
        match &current.kind {
            Kind::Object(members) => {
                current = &members.iter().find(|m| m.key == *segment)?.value;
            }
            _ => return None,
        }
    }
    Some(current)
}

/// Sets `key` in the object at `path` (creating missing objects) to `value`.
/// An existing entry is replaced in place; a new one is appended.
pub fn set(text: &str, path: &[&str], key: &str, value: &Value) -> Result<String, String> {
    let root = parse(text)?;
    let (unit, newline) = style(text);
    let Some(object) = find(&root, path) else {
        // Create the missing parent first.
        let (parent, last) = path.split_at(path.len() - 1);
        let mut wrapped = serde_json::Map::new();
        wrapped.insert(key.to_string(), value.clone());
        return set(text, parent, last[0], &Value::Object(wrapped));
    };
    let Kind::Object(members) = &object.kind else {
        return Err(format!("`{}` is not an object", path.join(".")));
    };
    if let Some(m) = members.iter().find(|m| m.key == key) {
        let base = line_indent(text, m.key_start);
        let rendered = render(value, &unit, &base, newline);
        return Ok(format!(
            "{}{rendered}{}",
            &text[..m.value.start],
            &text[m.value.end..]
        ));
    }
    let key_json = serde_json::to_string(key).expect("strings serialize");
    match members.last() {
        Some(last) => {
            let base = line_indent(text, last.key_start);
            let rendered = render(value, &unit, &base, newline);
            Ok(format!(
                "{},{newline}{base}{key_json}: {rendered}{}",
                &text[..last.value.end],
                &text[last.value.end..]
            ))
        }
        None => {
            let outer = line_indent(text, object.start);
            let base = format!("{outer}{unit}");
            let rendered = render(value, &unit, &base, newline);
            Ok(format!(
                "{}{{{newline}{base}{key_json}: {rendered}{newline}{outer}}}{}",
                &text[..object.start],
                &text[object.end..]
            ))
        }
    }
}

/// Removes `key` from the object at `path`. Returns `None` if absent.
pub fn remove(text: &str, path: &[&str], key: &str) -> Result<Option<String>, String> {
    let root = parse(text)?;
    let Some(object) = find(&root, path) else {
        return Ok(None);
    };
    let Kind::Object(members) = &object.kind else {
        return Ok(None);
    };
    let Some(i) = members.iter().position(|m| m.key == key) else {
        return Ok(None);
    };
    let out = if members.len() == 1 {
        format!("{}{{}}{}", &text[..object.start], &text[object.end..])
    } else if i > 0 {
        let prev_end = members[i - 1].value.end;
        format!("{}{}", &text[..prev_end], &text[members[i].value.end..])
    } else {
        format!(
            "{}{}",
            &text[..members[0].key_start],
            &text[members[1].key_start..]
        )
    };
    Ok(Some(out))
}

/// Appends `value` to the array at `path` (creating it when missing).
pub fn push(text: &str, path: &[&str], value: &Value) -> Result<String, String> {
    let root = parse(text)?;
    let (unit, newline) = style(text);
    let Some(array) = find(&root, path) else {
        let (parent, last) = path.split_at(path.len() - 1);
        return set(text, parent, last[0], &Value::Array(vec![value.clone()]));
    };
    let Kind::Array(items) = &array.kind else {
        return Err(format!("`{}` is not an array", path.join(".")));
    };
    match items.last() {
        Some(last) => {
            // Keep a one-line array on one line.
            let multiline = text[array.start..array.end].contains('\n');
            if multiline {
                let base = line_indent(text, last.start);
                let rendered = render(value, &unit, &base, newline);
                Ok(format!(
                    "{},{newline}{base}{rendered}{}",
                    &text[..last.end],
                    &text[last.end..]
                ))
            } else {
                Ok(format!(
                    "{}, {}{}",
                    &text[..last.end],
                    serde_json::to_string(value).expect("JSON values serialize"),
                    &text[last.end..]
                ))
            }
        }
        None => Ok(format!(
            "{}[{}]{}",
            &text[..array.start],
            serde_json::to_string(value).expect("JSON values serialize"),
            &text[array.end..]
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BASE: &str = "{\n    \"package\": { \"name\": \"a\", \"version\": \"1.0.0\" },\n    \"dependencies\": {\n        \"libstub\": { \"git\": \"x\" }\n    }\n}\n";

    #[test]
    fn adds_replaces_and_removes_preserving_style() {
        let added = set(
            BASE,
            &["dependencies"],
            "ace3",
            &json!({ "git": "y", "tag": "1" }),
        )
        .unwrap();
        assert_eq!(
            added,
            "{\n    \"package\": { \"name\": \"a\", \"version\": \"1.0.0\" },\n    \"dependencies\": {\n        \"libstub\": { \"git\": \"x\" },\n        \"ace3\": {\n            \"git\": \"y\",\n            \"tag\": \"1\"\n        }\n    }\n}\n"
        );
        let replaced = set(&added, &["dependencies"], "libstub", &json!("^1.0")).unwrap();
        assert!(replaced.contains("\"libstub\": \"^1.0\",\n"));
        let removed = remove(&replaced, &["dependencies"], "libstub")
            .unwrap()
            .unwrap();
        assert!(!removed.contains("libstub"));
        assert!(
            removed.contains("\"dependencies\": {\n        \"ace3\""),
            "{removed}"
        );
        let removed = remove(&removed, &["dependencies"], "ace3")
            .unwrap()
            .unwrap();
        assert!(removed.contains("\"dependencies\": {}"), "{removed}");
        assert_eq!(remove(&removed, &["dependencies"], "ace3").unwrap(), None);
        serde_json::from_str::<Value>(&removed).unwrap();
    }

    #[test]
    fn creates_missing_sections_and_keeps_crlf() {
        let text = "{\r\n  \"package\": { \"name\": \"a\", \"version\": \"1.0.0\" }\r\n}\r\n";
        let out = set(
            text,
            &["dev-dependencies"],
            "mock",
            &json!({ "path": "../m" }),
        )
        .unwrap();
        assert!(!out.replace("\r\n", "").contains('\n'), "{out:?}");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["dev-dependencies"]["mock"]["path"], "../m");
        let out = push(&out, &["addon-dependencies", "optional"], &json!("Details")).unwrap();
        let out = push(&out, &["addon-dependencies", "optional"], &json!("Masque")).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v["addon-dependencies"]["optional"],
            json!(["Details", "Masque"])
        );
    }
}
