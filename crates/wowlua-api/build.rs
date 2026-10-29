//! Takes the globals of a primitive type (`declare global NAME: string`,
//! with at most an `@onDemand("Addon")` line) out of each declaration file:
//! a table holds them, and the text the compiler parses has spaces in their
//! place (the same length, every other byte and line where it was). About
//! two thirds of the API's statements are such globals.

use std::fmt::Write as _;
use std::path::Path;

/// `shared` and every flavor of the flavor table (a test checks the list).
const FILES: &[&str] = &["shared", "camelot", "standard", "vanilla", "tbc"];
const PRIMS: &[&str] = &["any", "boolean", "number", "string", "table"];

fn ident(s: &str) -> Option<&str> {
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    let name = &s[..end];
    let first = name.chars().next()?;
    (first.is_ascii_alphabetic() || first == '_').then_some(name)
}

/// `declare global NAME: <primitive>` as the whole line: the name and type.
fn primitive_global(line: &str) -> Option<(&str, u8)> {
    let rest = line.strip_prefix("declare global ")?;
    let name = ident(rest)?;
    let ty = rest[name.len()..].strip_prefix(": ")?;
    let ty = PRIMS.iter().position(|p| *p == ty)?;
    Some((name, ty as u8))
}

/// `@onDemand("Addon")` as the whole line, with a plain identifier.
fn on_demand(line: &str) -> Option<&str> {
    let addon = line.strip_prefix("@onDemand(\"")?.strip_suffix("\")")?;
    (ident(addon) == Some(addon)).then_some(addon)
}

fn main() {
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    println!("cargo:rerun-if-changed=data");
    for name in FILES {
        let path = format!("data/{name}.d.wowlua");
        println!("cargo:rerun-if-changed={path}");
        let text = std::fs::read_to_string(&path).expect("the declaration file");
        // Lines with their terminators, and without (`\r\n` or `\n`).
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let body = |l: &str| -> usize {
            let l = l.strip_suffix('\n').unwrap_or(l);
            l.strip_suffix('\r').unwrap_or(l).len()
        };
        let content = |i: usize| &lines[i][..body(lines[i])];
        // The line before `i` that is code (not blank, not a comment).
        let code_before = |i: usize| {
            (0..i).rev().find(|&j| {
                let t = content(j).trim();
                !t.is_empty() && !t.starts_with("--")
            })
        };
        let mut blank = vec![false; lines.len()];
        let mut names = String::new();
        let mut types = String::new();
        let mut addons = String::new();
        let mut taken = std::collections::HashMap::<&str, usize>::new();
        let mut count = 0usize;
        for i in 0..lines.len() {
            let line = content(i);
            assert!(
                !line.starts_with("--!"),
                "{path}: a directive would move with the removed lines"
            );
            for prefix in ["declare global ", "declare function ", "declare namespace "] {
                if let Some(n) = line
                    .strip_prefix("@deprecated ")
                    .unwrap_or(line)
                    .strip_prefix(prefix)
                    .and_then(ident)
                {
                    *taken.entry(n).or_default() += 1;
                }
            }
            let Some((global, ty)) = primitive_global(line) else {
                continue;
            };
            // At most one decorator, `@onDemand`, on a line of its own.
            let mut addon = None;
            let prev = code_before(i);
            if let Some(p) = prev
                && content(p).trim_start().starts_with('@')
            {
                let Some(a) = on_demand(content(p)) else {
                    continue;
                };
                if code_before(p).is_some_and(|q| content(q).trim_start().starts_with('@')) {
                    continue;
                }
                addon = Some((p, a));
            }
            blank[i] = true;
            if let Some((p, a)) = addon {
                blank[p] = true;
                writeln!(addons, "    ({count}, {a:?}),").expect("write");
            }
            if count > 0 {
                names.push('\n');
            }
            names.push_str(global);
            types.push((b'0' + ty) as char);
            count += 1;
        }
        // Applying the table at the start of the file's turn equals applying
        // each line where it was only if no other declaration of the file
        // writes the same global.
        for global in names.split('\n').filter(|n| !n.is_empty()) {
            assert_eq!(
                taken[global], 1,
                "{path}: `{global}` is declared more than once"
            );
        }
        let mut padded = String::with_capacity(text.len());
        for (i, line) in lines.iter().enumerate() {
            if blank[i] {
                let n = body(line);
                padded.extend(std::iter::repeat_n(' ', n));
                padded.push_str(&line[n..]);
            } else {
                padded.push_str(line);
            }
        }
        assert_eq!(padded.len(), text.len());
        let dir = Path::new(&out);
        std::fs::write(dir.join(format!("{name}.d.wowlua")), padded).expect("write");
        let table = format!(
            "crate::Globals {{\n    names: {names:?},\n    types: b{types:?},\n    on_demand: &[\n{addons}    ],\n}}\n"
        );
        std::fs::write(dir.join(format!("{name}.globals.rs")), table).expect("write");
    }
}
