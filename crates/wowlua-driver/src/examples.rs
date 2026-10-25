//! Code examples in doc comments run as tests (spec Tooling §16.1.3,
//! decision T222): a fenced ```` ```wowlua ```` block in a `---` comment of
//! an addon source becomes a test of a test file made for that source,
//! `<path>.examples.test.wowlua`, which no disk holds. The made file is the
//! source's lines blank but the examples', each at its own line and column
//! (the `---` blanked), so a diagnostic or a failure in an example points
//! at the comment it is written in. A block marked `wowlua norun` (or of
//! another language) is left alone.

use wowlua_span::SourceDb;
use wowlua_syntax::view::StmtKind;

/// The suffix of a made examples file (`src/money.wowlua` →
/// `src/money.examples.test.wowlua`).
pub const SUFFIX: &str = ".examples.test.wowlua";

/// The path of the examples file of source `rel` (`.wowlua`).
pub fn examples_path(rel: &str) -> Option<String> {
    Some(format!("{}{SUFFIX}", rel.strip_suffix(".wowlua")?))
}

/// The source an examples file is made from (`…examples.test.wowlua` →
/// `….wowlua`), else the path as it is.
pub fn source_of(rel: &str) -> String {
    match rel.strip_suffix(SUFFIX) {
        Some(stem) => format!("{stem}.wowlua"),
        None => rel.to_string(),
    }
}

/// One example: the lines of its fences and the declaration it documents.
struct Example {
    open: usize,
    close: usize,
    name: String,
}

/// The text after a doc comment's `---` (the line's), if it is one.
fn doc_text(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let rest = t.strip_prefix("---")?;
    // (`----` is no doc comment)
    if rest.starts_with('-') {
        return None;
    }
    Some(rest)
}

/// The name a line declares (`export function add(…)` → `add`,
/// `function Counter:bump()` → `Counter:bump`, `export class Counter` →
/// `Counter`).
fn declared(line: &str) -> Option<String> {
    let mut words = line.split_whitespace().peekable();
    while let Some(w) = words.peek() {
        if matches!(
            *w,
            "export"
                | "local"
                | "abstract"
                | "static"
                | "private"
                | "protected"
                | "public"
                | "overload"
                | "declare"
        ) {
            words.next();
        } else {
            break;
        }
    }
    let first = words.next()?;
    let name = match first {
        "function" | "class" | "interface" | "enum" | "type" | "const" => words.next()?,
        // a field or a method of a class body (`function` left out)
        other => other,
    };
    let name: String = name
        .chars()
        .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':'))
        .collect();
    (!name.is_empty()).then_some(name)
}

/// The examples of `text`.
fn examples(lines: &[&str]) -> Vec<Example> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let open = doc_text(lines[i])
            .map(str::trim)
            .filter(|t| t.starts_with("```"));
        let Some(fence) = open else {
            i += 1;
            continue;
        };
        let info = fence.trim_start_matches('`').trim();
        let mut words = info.split_whitespace();
        let runs = words.next() == Some("wowlua") && !words.any(|w| w == "norun" || w == "ignore");
        // its closing fence, every line between a doc comment's
        let mut j = i + 1;
        let mut whole = true;
        while j < lines.len() {
            match doc_text(lines[j]) {
                Some(t) if t.trim() == "```" => break,
                Some(_) => j += 1,
                None => {
                    whole = false;
                    break;
                }
            }
        }
        if j >= lines.len() || !whole {
            i = j.max(i + 1);
            continue;
        }
        if runs {
            // the declaration the comment is above
            let mut k = j + 1;
            while k < lines.len() {
                let t = lines[k].trim_start();
                if t.starts_with("--") || t.starts_with('@') || t.is_empty() {
                    k += 1;
                } else {
                    break;
                }
            }
            let name = lines
                .get(k)
                .and_then(|l| declared(l))
                .unwrap_or_else(|| "file".into());
            out.push(Example {
                open: i,
                close: j,
                name,
            });
        }
        i = j + 1;
    }
    out
}

/// The examples file of the addon source `rel` with `text`: `None` without
/// an example. `module` is how a test file imports the source (`/money`),
/// `None` for one no module path names (its examples see its globals only).
pub fn examples_file(text: &str, module: Option<&str>) -> Option<String> {
    if !text.contains("```wowlua") {
        return None;
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let found = examples(&lines);
    if found.is_empty() {
        return None;
    }
    let mut out: Vec<String> = vec![String::new(); lines.len()];
    // what an example sees: the module's exports and `expect`
    let mut exports: Vec<String> = Vec::new();
    if module.is_some() {
        let mut db = SourceDb::new();
        let file = db.add("examples", String::new());
        let parse = wowlua_syntax::parse(text.as_bytes(), file);
        for s in parse.view().block().stmts() {
            if let StmtKind::Export(inner) = s.kind() {
                exports.extend(wowlua_sema::exported_names_of(inner));
            }
        }
        exports.sort();
        exports.dedup();
    }
    let mut imports = String::from("import test as __wowlua_test");
    if !exports.iter().any(|e| e == "expect") {
        imports.push_str(", expect");
    }
    imports.push_str(" from \"wowlua:test\"; ");
    if let (Some(m), false) = (module, exports.is_empty()) {
        imports.push_str(&format!("import {} from \"{m}\"; ", exports.join(", ")));
    }
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (n, e) in found.iter().enumerate() {
        let count = seen.entry(e.name.clone()).or_insert(0);
        *count += 1;
        let name = if *count == 1 {
            format!("{} example", e.name)
        } else {
            format!("{} example {count}", e.name)
        };
        let lead = if n == 0 { imports.as_str() } else { "" };
        out[e.open] = format!(
            "{lead}@__wowlua_test({}) local function __wowlua_example_{}()",
            quoted(&name),
            n + 1
        );
        for (l, line) in lines.iter().enumerate().take(e.close).skip(e.open + 1) {
            // `--- code` → `    code`: the code at its own column
            let indent = line.len() - line.trim_start().len();
            let after = &line[indent + 3..];
            let pad = indent + 3 + usize::from(after.starts_with(' '));
            out[l] = format!(
                "{}{}",
                " ".repeat(pad),
                line[pad.min(line.len())..].trim_end_matches('\r')
            );
        }
        out[e.close] = "end".to_string();
    }
    // no lints: an example uses what it needs of the exports
    if found[0].open > 0 {
        out[0] = "--!nolint".to_string();
    }
    Some(out.join("\n"))
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "-- Money.\n\n--- Adds two amounts.\n---\n--- ```wowlua\n--- expect(add(1, 2)).toBe(3)\n--- ```\nexport function add(a: number, b: number): number\n    return a + b\nend\n\nexport class Purse\n    --- ```wowlua\n    --- local p = Purse.new()\n    --- expect(p:total()).toBe(0)\n    --- ```\n    --- ```wowlua norun\n    --- p:spend(1)\n    --- ```\n    function total(): number\n        return 0\n    end\nend\n";

    #[test]
    fn examples_keep_their_lines_and_columns() {
        let made = examples_file(SRC, Some("/money")).expect("examples");
        let lines: Vec<&str> = made.split('\n').collect();
        assert_eq!(lines.len(), SRC.split('\n').count());
        assert_eq!(lines[0], "--!nolint");
        assert!(lines[4].starts_with("import test as __wowlua_test, expect from \"wowlua:test\"; import Purse, add from \"/money\"; @__wowlua_test(\"add example\")"), "{}", lines[4]);
        assert_eq!(lines[5], "    expect(add(1, 2)).toBe(3)");
        assert_eq!(lines[6], "end");
        assert!(
            lines[12].starts_with("@__wowlua_test(\"total example\")"),
            "{}",
            lines[12]
        );
        assert_eq!(lines[13], "        local p = Purse.new()");
        // `norun` is left alone, as is everything else
        assert_eq!(lines[17], "");
        assert_eq!(lines[8], "");
        // it parses
        let mut db = SourceDb::new();
        let file = db.add("x", String::new());
        assert!(wowlua_syntax::parses(made.as_bytes(), file), "{made}");
    }

    #[test]
    fn a_source_without_examples_makes_none() {
        assert!(examples_file("--- Adds.\nexport function add() end\n", Some("/m")).is_none());
        assert_eq!(
            source_of("src/money.examples.test.wowlua"),
            "src/money.wowlua"
        );
        assert_eq!(
            examples_path("src/money.wowlua").as_deref(),
            Some("src/money.examples.test.wowlua")
        );
    }
}
