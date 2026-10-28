//! The recordings wowlua's dev window exports in the game (spec Debugging
//! §7.7.9): found in `WowluaDevDB_<Addon>` in the installation's
//! SavedVariables, read as JSON, their locations mapped to the source.
//! `wowlua prof report` and wowlua studio's profiler read them.

use crate::project::Project;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// One SavedVariables file with exported recordings.
pub struct Found {
    /// The client folder (`_retail_`).
    pub client: String,
    pub account: String,
    pub path: PathBuf,
    /// The exported recordings (`format` 1).
    pub exports: Vec<Value>,
}

/// The SavedVariables name of an addon's dev data (as dev builds add it to
/// the TOC).
pub fn db_name(addon: &str) -> String {
    let safe: String = addon
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("WowluaDevDB_{safe}")
}

fn folder_matches(folder: &str, wanted: &str) -> bool {
    folder
        .trim_matches('_')
        .eq_ignore_ascii_case(wanted.trim_matches('_'))
}

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = crate::host::read_dir(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| crate::host::is_dir(p))
        .collect();
    out.sort();
    out
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Every client folder and account of the installation with exported
/// recordings of `db` (the `client` and `account` given only), and a
/// warning for each file that can't be read.
pub fn find(
    wow: &Path,
    db: &str,
    client: Option<&str>,
    account: Option<&str>,
) -> (Vec<Found>, Vec<String>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    for folder in sorted_dirs(wow) {
        let client_name = file_name(&folder);
        if client.is_some_and(|c| !folder_matches(&client_name, c)) {
            continue;
        }
        for dir in sorted_dirs(&folder.join("WTF").join("Account")) {
            let account_name = file_name(&dir);
            if account_name.eq_ignore_ascii_case("SavedVariables")
                || account.is_some_and(|a| !a.eq_ignore_ascii_case(&account_name))
            {
                continue;
            }
            let path = dir.join("SavedVariables").join(format!("{db}.lua"));
            let Ok(bytes) = crate::host::read(&path) else {
                continue;
            };
            let value = match parse_saved(&bytes) {
                Ok(vars) => vars.into_iter().find(|(n, _)| n == db).map(|(_, v)| v),
                Err(e) => {
                    warnings.push(format!("cannot read `{}`: {e}", path.display()));
                    continue;
                }
            };
            let exports: Vec<Value> = value
                .as_ref()
                .and_then(|v| v.get("exports"))
                .map(list)
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.get("format").and_then(Value::as_f64) == Some(1.0))
                .collect();
            if !exports.is_empty() {
                out.push(Found {
                    client: client_name.clone(),
                    account: account_name,
                    path,
                    exports,
                });
            }
        }
    }
    (out, warnings)
}

/// The items of a Lua array (an array, or an object of numbered keys).
pub fn list(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.iter().filter(|v| !v.is_null()).cloned().collect(),
        Value::Object(o) => o.values().cloned().collect(),
        _ => Vec::new(),
    }
}

// ------------------------------------------------------------ locations --

/// The recording's game locations mapped to source lines as `wowlua trace`
/// does, with the build id it was recorded with: the slow calls' stacks
/// (each line followed by its `→ src/x.wowlua:12 in name`), and locations
/// of built files.
pub fn map_locations(project: &Project, rec: &mut Value) {
    let build = rec.get("build").and_then(Value::as_str).map(str::to_string);
    // Every text to map, by where it goes.
    let mut texts: Vec<String> = Vec::new();
    let mut slots: Vec<(&'static str, usize, &'static str)> = Vec::new();
    if let Some(Value::Array(slow)) = rec.get("slow") {
        for (i, s) in slow.iter().enumerate() {
            for field in ["stack", "location"] {
                if let Some(t) = s.get(field).and_then(Value::as_str)
                    && !t.is_empty()
                {
                    texts.push(t.to_string());
                    slots.push(("slow", i, field));
                }
            }
        }
    }
    if texts.is_empty() {
        return;
    }
    const SEP: &str = "\u{1}wowlua-prof-report\u{1}";
    let input = texts.join(&format!("\n{SEP}\n"));
    let decoded = crate::trace::decode(project, &input, build.as_deref());
    let parts: Vec<String> = decoded
        .split(&format!("{SEP}\n"))
        .map(|p| p.trim_end_matches('\n').to_string())
        .collect();
    if parts.len() != texts.len() {
        return;
    }
    for ((list, i, field), text) in slots.into_iter().zip(parts) {
        let Some(entry) = rec
            .get_mut(list)
            .and_then(|l| l.get_mut(i))
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        if field == "location" {
            // A built file's location: its source line.
            if let Some(mapped) = text.lines().find_map(|l| l.trim().strip_prefix("→ ")) {
                let place = mapped.split(" in ").next().unwrap_or(mapped);
                entry.insert("location".into(), Value::String(place.to_string()));
            }
        } else {
            entry.insert(field.into(), Value::String(text));
        }
    }
}

// ------------------------------------------------------ SavedVariables --

/// A Lua key as the client writes it.
#[derive(Debug)]
enum Key {
    Int(i64),
    Str(String),
    Other(String),
}

/// The variables of a SavedVariables file (`Name = value` statements, the
/// data part of Lua the client writes), as JSON: tables with the keys 1..n
/// are arrays (holes are null), other tables objects.
pub fn parse_saved(bytes: &[u8]) -> Result<Vec<(String, Value)>, String> {
    let mut p = Parser { s: bytes, i: 0 };
    let mut out = Vec::new();
    loop {
        p.skip();
        if p.i >= p.s.len() {
            return Ok(out);
        }
        let name = p.name().ok_or_else(|| p.error("a variable name"))?;
        p.skip();
        p.expect(b'=')?;
        let value = p.value()?;
        out.push((name, value));
        p.skip();
        if p.peek() == Some(b';') {
            p.i += 1;
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn line(&self) -> usize {
        1 + self.s[..self.i.min(self.s.len())]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
    }

    fn error(&self, wanted: &str) -> String {
        format!("line {}: expected {wanted}", self.line())
    }

    /// Whitespace and comments.
    fn skip(&mut self) {
        loop {
            while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"--") {
                self.i += 2;
                if let Some(level) = self.long_open() {
                    self.long_body(level);
                } else {
                    while self.peek().is_some_and(|b| b != b'\n') {
                        self.i += 1;
                    }
                }
            } else {
                return;
            }
        }
    }

    /// `[[` or `[==[` at the cursor: consumed, its level.
    fn long_open(&mut self) -> Option<usize> {
        let rest = &self.s[self.i..];
        if rest.first() != Some(&b'[') {
            return None;
        }
        let level = rest[1..].iter().take_while(|&&b| b == b'=').count();
        (rest.get(1 + level) == Some(&b'[')).then(|| {
            self.i += 2 + level;
            level
        })
    }

    /// A long bracket's text up to its closing bracket.
    fn long_body(&mut self, level: usize) -> Vec<u8> {
        let close: Vec<u8> = std::iter::once(b']')
            .chain(std::iter::repeat_n(b'=', level))
            .chain(std::iter::once(b']'))
            .collect();
        let start = self.i;
        while self.i < self.s.len() && !self.s[self.i..].starts_with(&close) {
            self.i += 1;
        }
        let mut text = self.s[start..self.i].to_vec();
        self.i = (self.i + close.len()).min(self.s.len());
        if text.first() == Some(&b'\n') {
            text.remove(0);
        }
        text
    }

    fn expect(&mut self, b: u8) -> Result<(), String> {
        if self.peek() == Some(b) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.error(&format!("`{}`", b as char)))
        }
    }

    fn name(&mut self) -> Option<String> {
        let start = self.i;
        if !self
            .peek()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        {
            return None;
        }
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            self.i += 1;
        }
        Some(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    fn value(&mut self) -> Result<Value, String> {
        self.skip();
        match self.peek() {
            Some(b'{') => self.table(),
            Some(b'"' | b'\'') => Ok(Value::String(self.string()?)),
            Some(b'[') => match self.long_open() {
                Some(level) => Ok(Value::String(
                    String::from_utf8_lossy(&self.long_body(level)).into_owned(),
                )),
                None => Err(self.error("a value")),
            },
            Some(b'-' | b'.' | b'0'..=b'9') => self.number(),
            _ => match self.name().as_deref() {
                Some("true") => Ok(Value::Bool(true)),
                Some("false") => Ok(Value::Bool(false)),
                Some("nil" | "inf" | "nan") => Ok(Value::Null),
                _ => Err(self.error("a value")),
            },
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let negative = self.peek() == Some(b'-');
        if negative {
            self.i += 1;
            self.skip();
        }
        let start = self.i;
        while self.peek().is_some_and(|b| {
            b.is_ascii_alphanumeric()
                || b == b'.'
                || ((b == b'-' || b == b'+')
                    && matches!(self.s.get(self.i.wrapping_sub(1)), Some(b'e' | b'E')))
        }) {
            self.i += 1;
        }
        let text = String::from_utf8_lossy(&self.s[start..self.i]).to_ascii_lowercase();
        let n = if let Some(hex) = text.strip_prefix("0x") {
            i64::from_str_radix(hex, 16).ok().map(|n| n as f64)
        } else if text == "inf" || text == "nan" {
            None
        } else {
            text.parse::<f64>().ok()
        };
        let Some(n) = n else {
            // `-nan`, `inf`: no number JSON can hold.
            return if text.chars().all(|c| c.is_ascii_alphabetic()) {
                Ok(Value::Null)
            } else {
                Err(self.error("a number"))
            };
        };
        let n = if negative { -n } else { n };
        Ok(json_number(n))
    }

    fn string(&mut self) -> Result<String, String> {
        let quote = self.s[self.i];
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(b) = self.peek() else {
                return Err(self.error("the end of a string"));
            };
            self.i += 1;
            if b == quote {
                break;
            }
            if b != b'\\' {
                out.push(b);
                continue;
            }
            let Some(e) = self.peek() else {
                return Err(self.error("an escape"));
            };
            self.i += 1;
            match e {
                b'n' | b'\n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                b'a' => out.push(7),
                b'b' => out.push(8),
                b'f' => out.push(12),
                b'v' => out.push(11),
                b'\r' => {
                    out.push(b'\n');
                    if self.peek() == Some(b'\n') {
                        self.i += 1;
                    }
                }
                b'0'..=b'9' => {
                    let mut n = u32::from(e - b'0');
                    for _ in 0..2 {
                        match self.peek() {
                            Some(d @ b'0'..=b'9') => {
                                n = n * 10 + u32::from(d - b'0');
                                self.i += 1;
                            }
                            _ => break,
                        }
                    }
                    out.push(n.min(255) as u8);
                }
                other => out.push(other),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn table(&mut self) -> Result<Value, String> {
        self.i += 1;
        let mut entries: Vec<(Key, Value)> = Vec::new();
        let mut next = 1i64;
        loop {
            self.skip();
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                None => return Err(self.error("`}`")),
                _ => {}
            }
            let save = self.i;
            let key = if self.peek() == Some(b'[') && self.long_open().is_none() {
                self.i += 1;
                let k = self.value()?;
                self.skip();
                self.expect(b']')?;
                self.skip();
                self.expect(b'=')?;
                Some(match k {
                    Value::String(s) => Key::Str(s),
                    Value::Number(n) if n.as_i64().is_some() => Key::Int(n.as_i64().unwrap_or(0)),
                    other => Key::Other(other.to_string()),
                })
            } else {
                self.i = save;
                // `name = value`, or a positional value.
                let named = self.name().and_then(|n| {
                    self.skip();
                    (self.peek() == Some(b'=') && self.s.get(self.i + 1) != Some(&b'=')).then(
                        || {
                            self.i += 1;
                            n
                        },
                    )
                });
                match named {
                    Some(n) => Some(Key::Str(n)),
                    None => {
                        self.i = save;
                        None
                    }
                }
            };
            let value = self.value()?;
            let key = key.unwrap_or_else(|| {
                next += 1;
                Key::Int(next - 1)
            });
            if !value.is_null() {
                entries.push((key, value));
            }
            self.skip();
            if matches!(self.peek(), Some(b',' | b';')) {
                self.i += 1;
            }
        }
        Ok(table_value(entries))
    }
}

fn json_number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

fn table_value(entries: Vec<(Key, Value)>) -> Value {
    let ints: Option<Vec<i64>> = entries
        .iter()
        .map(|(k, _)| match k {
            Key::Int(i) if *i >= 1 => Some(*i),
            _ => None,
        })
        .collect();
    if let Some(ints) = ints
        && !entries.is_empty()
    {
        let max = ints.iter().copied().max().unwrap_or(0);
        if max as usize <= 2 * entries.len() + 8 {
            let mut items = vec![Value::Null; max as usize];
            for (k, v) in entries {
                if let Key::Int(i) = k {
                    items[i as usize - 1] = v;
                }
            }
            return Value::Array(items);
        }
    }
    let mut map = Map::new();
    for (k, v) in entries {
        let k = match k {
            Key::Int(i) => i.to_string(),
            Key::Str(s) | Key::Other(s) => s,
        };
        map.insert(k, v);
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_the_client_writes() {
        let text = br#"
WowluaDevDB_Probe = {
	["exports"] = {
		{
			["name"] = "pull 1",
			["detail"] = "Ulgrax",
			["duration"] = 12.5,
			["frames"] = 600,
			["neg"] = -3,
			["small"] = 1.5e-05,
			["stack"] = "a\nb \"c\" \\ d\
e",
			["seconds"] = {
				["frames"] = {
					60, -- [1]
					59, -- [2]
				},
			},
			["functions"] = {
			},
			[2] = true,
		}, -- [1]
	},
	["settings"] = {
		["callPaths"] = true,
	},
}
Other = "x"
"#;
        let vars = parse_saved(text).unwrap();
        assert_eq!(vars.len(), 2);
        let db = &vars[0].1;
        let rec = &db["exports"][0];
        assert_eq!(rec["name"], "pull 1");
        assert_eq!(rec["duration"], 12.5);
        assert_eq!(rec["frames"], 600);
        assert_eq!(rec["neg"], -3);
        assert_eq!(rec["small"], 1.5e-05);
        assert_eq!(rec["stack"], "a\nb \"c\" \\ d\ne");
        assert_eq!(rec["seconds"]["frames"], serde_json::json!([60, 59]));
        assert_eq!(rec["functions"], serde_json::json!({}));
        assert_eq!(rec["2"], true);
        assert_eq!(db["settings"]["callPaths"], true);
        assert_eq!(vars[1], ("Other".to_string(), Value::from("x")));
    }

    #[test]
    fn arrays_with_holes_and_errors() {
        let vars = parse_saved(b"X = { [1] = 1, [3] = 3 }").unwrap();
        assert_eq!(vars[0].1, serde_json::json!([1, null, 3]));
        let vars = parse_saved(b"X = { [1] = 1, [1000] = 3 }").unwrap();
        assert_eq!(vars[0].1, serde_json::json!({ "1": 1, "1000": 3 }));
        let err = parse_saved(b"X = {\n[\"a\"] = }").unwrap_err();
        assert!(err.starts_with("line 2"), "{err}");
    }

    #[test]
    fn names() {
        assert_eq!(db_name("My-Addon"), "WowluaDevDB_My_Addon");
    }
}
