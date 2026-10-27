//! Code highlighting for READMEs, as wowlua.dev's (`lib/highlight.ts`): the
//! design system's syntax classes (`tok-k` keyword, `tok-t` type, `tok-s`
//! string, `tok-n` number, `tok-c` comment, `tok-d` decorator, `tok-f`
//! function name, `tok-p` punctuation), one `wl-code__line` per line.

use crate::html::esc;

const KEYWORDS: &[&str] = &[
    "and",
    "break",
    "do",
    "else",
    "elseif",
    "end",
    "false",
    "for",
    "function",
    "goto",
    "if",
    "in",
    "local",
    "nil",
    "not",
    "or",
    "repeat",
    "return",
    "then",
    "true",
    "until",
    "while",
    // wowlua's
    "abstract",
    "as",
    "class",
    "const",
    "constructor",
    "continue",
    "declare",
    "enum",
    "export",
    "extends",
    "from",
    "implements",
    "import",
    "interface",
    "keyof",
    "new",
    "override",
    "private",
    "protected",
    "public",
    "readonly",
    "self",
    "static",
    "super",
    "type",
    "typeof",
];

const TYPES: &[&str] = &[
    "any", "boolean", "never", "number", "string", "table", "unknown", "void", "thread", "userdata",
];

/// A run of text and its class.
type Piece = (Option<&'static str>, String);

/// HTML for a code block's lines.
pub fn highlight(code: &str, lang: &str) -> String {
    let pieces = match lang.to_ascii_lowercase().as_str() {
        "wowlua" | "lua" => lua(code),
        "json" | "jsonc" => json(code),
        "sh" | "bash" | "shell" | "powershell" | "ps1" => shell(code),
        _ => vec![(None, code.to_string())],
    };
    lines(&pieces)
}

/// Wraps each line (a piece across lines is split), keeping empty lines'
/// height with a space.
fn lines(pieces: &[Piece]) -> String {
    let mut out = String::new();
    let mut line = String::new();
    let mut empty = true;
    let flush = |line: &mut String, empty: &mut bool, out: &mut String| {
        out.push_str("<span class=\"wl-code__line\">");
        out.push_str(line);
        if *empty {
            out.push(' ');
        }
        out.push_str("</span>");
        line.clear();
        *empty = true;
    };
    for (class, text) in pieces {
        for (i, part) in text.split('\n').enumerate() {
            if i > 0 {
                flush(&mut line, &mut empty, &mut out);
            }
            if part.is_empty() {
                continue;
            }
            empty = false;
            match class {
                Some(c) => line.push_str(&format!("<span class=\"{c}\">{}</span>", esc(part))),
                None => line.push_str(&esc(part)),
            }
        }
    }
    flush(&mut line, &mut empty, &mut out);
    out
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The length of a long bracket (`[[`, `[==[`) at `i`, and its level.
fn long_open(s: &[u8], i: usize) -> Option<(usize, usize)> {
    if s.get(i) != Some(&b'[') {
        return None;
    }
    let mut j = i + 1;
    while s.get(j) == Some(&b'=') {
        j += 1;
    }
    (s.get(j) == Some(&b'[')).then_some((j + 1 - i, j - i - 1))
}

/// The end of a long bracket's contents opened at `from` (after its `]=*]`).
fn long_close(s: &[u8], from: usize, level: usize) -> Option<usize> {
    let mut i = from;
    while i < s.len() {
        if s[i] == b']' {
            let mut j = i + 1;
            while j < s.len() && s[j] == b'=' {
                j += 1;
            }
            if j - i - 1 == level && s.get(j) == Some(&b']') {
                return Some(j + 1);
            }
        }
        i += 1;
    }
    None
}

/// A quoted string at `i` (`"…"`, `'…'`, or a backtick string across lines).
fn quoted(s: &[u8], i: usize) -> Option<usize> {
    let q = *s.get(i)?;
    if q != b'"' && q != b'\'' && q != b'`' {
        return None;
    }
    let mut j = i + 1;
    while j < s.len() {
        match s[j] {
            b'\\' => j += 2,
            b'\n' if q != b'`' => return Some(j),
            c if c == q => return Some(j + 1),
            _ => j += 1,
        }
    }
    Some(s.len())
}

fn digits(s: &[u8], mut j: usize, extra: impl Fn(u8) -> bool) -> usize {
    while j < s.len() && (s[j].is_ascii_digit() || extra(s[j])) {
        j += 1;
    }
    j
}

/// A number at `i`: `0x…`, or digits with an optional fraction and exponent.
fn number(s: &[u8], i: usize, underscores: bool) -> Option<usize> {
    if !s.get(i)?.is_ascii_digit() {
        return None;
    }
    if underscores && s[i] == b'0' && matches!(s.get(i + 1), Some(b'x' | b'X')) {
        let j = digits(s, i + 2, |b| b.is_ascii_hexdigit() || b == b'_');
        if j > i + 2 {
            return Some(j);
        }
    }
    let sep = |b: u8| underscores && b == b'_';
    let mut j = digits(s, i, sep);
    if s.get(j) == Some(&b'.') && s.get(j + 1).is_some_and(|b| b.is_ascii_digit() || sep(*b)) {
        j = digits(s, j + 1, sep);
    }
    if matches!(s.get(j), Some(b'e' | b'E')) {
        let k = if matches!(s.get(j + 1), Some(b'+' | b'-')) {
            j + 2
        } else {
            j + 1
        };
        if s.get(k).is_some_and(u8::is_ascii_digit) {
            j = digits(s, k, |_| false);
        }
    }
    Some(j)
}

/// Pushes `text`, joining runs of plain text.
fn push(out: &mut Vec<Piece>, class: Option<&'static str>, text: &str) {
    if class.is_none()
        && let Some((None, last)) = out.last_mut()
    {
        last.push_str(text);
        return;
    }
    out.push((class, text.to_string()));
}

fn lua(code: &str) -> Vec<Piece> {
    let s = code.as_bytes();
    let mut out = Vec::new();
    let mut prev = String::new();
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        let (class, end): (Option<&'static str>, usize) = if rest.starts_with(b"--") {
            match long_open(s, i + 2) {
                Some((open, level)) => (
                    Some("tok-c"),
                    long_close(s, i + 2 + open, level).unwrap_or(s.len()),
                ),
                None => (
                    Some("tok-c"),
                    s[i..]
                        .iter()
                        .position(|&b| b == b'\n')
                        .map_or(s.len(), |p| i + p),
                ),
            }
        } else if let Some((open, level)) = long_open(s, i) {
            (
                Some("tok-s"),
                long_close(s, i + open, level).unwrap_or(s.len()),
            )
        } else if let Some(end) = quoted(s, i) {
            (Some("tok-s"), end)
        } else if let Some(end) = number(s, i, true) {
            (Some("tok-n"), end)
        } else if s[i] == b'@'
            && s.get(i + 1)
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        {
            let mut j = i + 1;
            while j < s.len() && (is_word(s[j]) || s[j] == b'.') {
                j += 1;
            }
            (Some("tok-d"), j)
        } else if s[i].is_ascii_alphabetic() || s[i] == b'_' {
            let mut j = i;
            while j < s.len() && is_word(s[j]) {
                j += 1;
            }
            let word = &code[i..j];
            let class = if KEYWORDS.contains(&word) {
                Some("tok-k")
            } else if TYPES.contains(&word) {
                Some("tok-t")
            } else if prev == "function" {
                Some("tok-f")
            } else if word.starts_with(|c: char| c.is_ascii_uppercase()) {
                Some("tok-t")
            } else {
                None
            };
            (class, j)
        } else if rest.starts_with(b"...") {
            (Some("tok-p"), i + 3)
        } else if rest.starts_with(b"..") {
            (Some("tok-p"), i + 2)
        } else if b":=<>~?|&+-*/%#^,;{}()[]".contains(&s[i]) {
            (Some("tok-p"), i + 1)
        } else {
            let len = code[i..].chars().next().map_or(1, char::len_utf8);
            (None, i + len)
        };
        let text = &code[i..end];
        if text.bytes().any(is_word) {
            prev = text.to_string();
        }
        push(&mut out, class, text);
        i = end;
    }
    out
}

fn json(code: &str) -> Vec<Piece> {
    let s = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let (class, end): (Option<&'static str>, usize) = if s[i] == b'"' {
            let end = quoted(s, i).unwrap_or(s.len());
            let mut k = end;
            while k < s.len() && s[k].is_ascii_whitespace() {
                k += 1;
            }
            (
                Some(if s.get(k) == Some(&b':') {
                    "tok-t"
                } else {
                    "tok-s"
                }),
                end,
            )
        } else if let Some(end) = (s[i] == b'-')
            .then(|| number(s, i + 1, false))
            .flatten()
            .or_else(|| number(s, i, false))
        {
            (Some("tok-n"), end)
        } else if let Some(w) = ["true", "false", "null"].iter().find(|w| {
            s[i..].starts_with(w.as_bytes())
                && (i == 0 || !is_word(s[i - 1]))
                && !s.get(i + w.len()).copied().is_some_and(is_word)
        }) {
            (Some("tok-k"), i + w.len())
        } else if s[i..].starts_with(b"//") {
            (
                Some("tok-c"),
                s[i..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(s.len(), |p| i + p),
            )
        } else if b"{}[]:,".contains(&s[i]) {
            (Some("tok-p"), i + 1)
        } else {
            let len = code[i..].chars().next().map_or(1, char::len_utf8);
            (None, i + len)
        };
        push(&mut out, class, &code[i..end]);
        i = end;
    }
    out
}

fn shell(code: &str) -> Vec<Piece> {
    let s = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let (class, end): (Option<&'static str>, usize) = if s[i] == b'#' {
            (
                Some("tok-c"),
                s[i..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(s.len(), |p| i + p),
            )
        } else if s[i] == b'"' || s[i] == b'\'' {
            (Some("tok-s"), quoted(s, i).unwrap_or(s.len()))
        } else if s[i] == b'-'
            && i > 0
            && s[i - 1].is_ascii_whitespace()
            && (s.get(i + 1).is_some_and(u8::is_ascii_alphabetic)
                || (s.get(i + 1) == Some(&b'-')
                    && s.get(i + 2).is_some_and(u8::is_ascii_alphabetic)))
        {
            let mut j = i + 1;
            while j < s.len() && (is_word(s[j]) || s[j] == b'-') {
                j += 1;
            }
            (Some("tok-t"), j)
        } else if b"|&;><".contains(&s[i]) {
            (Some("tok-p"), i + 1)
        } else {
            let len = code[i..].chars().next().map_or(1, char::len_utf8);
            (None, i + len)
        };
        push(&mut out, class, &code[i..end]);
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wowlua_lines_and_classes() {
        let html = highlight(
            "import lfg from \"foreverlfg\"\n\nlocal n = 1 -- one",
            "wowlua",
        );
        assert_eq!(
            html,
            "<span class=\"wl-code__line\"><span class=\"tok-k\">import</span> lfg <span class=\"tok-k\">from</span> <span class=\"tok-s\">&quot;foreverlfg&quot;</span></span>\
             <span class=\"wl-code__line\"> </span>\
             <span class=\"wl-code__line\"><span class=\"tok-k\">local</span> n <span class=\"tok-p\">=</span> <span class=\"tok-n\">1</span> <span class=\"tok-c\">-- one</span></span>"
        );
        assert!(highlight("function go() end", "lua").contains("<span class=\"tok-f\">go</span>"));
        assert!(highlight("--[[a\nb]]", "lua").contains("<span class=\"tok-c\">b]]</span>"));
    }

    #[test]
    fn json_keys_and_values() {
        let html = highlight("{ \"a\": \"^0.1\", \"b\": true }", "json");
        assert!(html.contains("<span class=\"tok-t\">&quot;a&quot;</span>"));
        assert!(html.contains("<span class=\"tok-s\">&quot;^0.1&quot;</span>"));
        assert!(html.contains("<span class=\"tok-k\">true</span>"));
    }
}
