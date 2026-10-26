//! Explanations of the diagnostic codes: what a code means, how to fix it,
//! an example that reports it and one that compiles, and related codes.
//! `wowlua explain` prints them and the website shows them on each code's
//! page. They are written by hand in `explanations/*.md`, one file per area
//! (see `explanations/README.md` for the format); `codes.rs` stays generated
//! from the spec.

use std::collections::HashMap;
use std::sync::OnceLock;

/// One code's explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    /// The area's short name, as the file's heading gives it (`types`).
    pub area: &'static str,
    /// What the code means and how to fix it, in Markdown.
    pub text: String,
    /// Examples in their order in the file.
    pub examples: Vec<Example>,
    /// Related codes, from the section's `Related:` line.
    pub related: Vec<String>,
}

/// An example: one or more files that report the code, or that compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example {
    pub compiles: bool,
    pub files: Vec<ExampleFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExampleFile {
    /// The file's path in the package: the block's `title=`, else
    /// `src/main.wowlua` (`tests/main.test.wowlua` when it imports
    /// `wowlua:test` or `wowlua:mock`).
    pub path: String,
    /// The fence's language (`wowlua`, `lua`, `json`).
    pub lang: String,
    pub text: String,
}

/// An area's file: its range (`x2xxx`), short name and source.
struct AreaFile {
    range: &'static str,
    source: &'static str,
}

const FILES: &[AreaFile] = &[
    AreaFile {
        range: "x01xx",
        source: include_str!("../explanations/x01xx.md"),
    },
    AreaFile {
        range: "x1xxx",
        source: include_str!("../explanations/x1xxx.md"),
    },
    AreaFile {
        range: "x2xxx",
        source: include_str!("../explanations/x2xxx.md"),
    },
    AreaFile {
        range: "x3xxx",
        source: include_str!("../explanations/x3xxx.md"),
    },
    AreaFile {
        range: "x4xxx",
        source: include_str!("../explanations/x4xxx.md"),
    },
    AreaFile {
        range: "x5xxx",
        source: include_str!("../explanations/x5xxx.md"),
    },
    AreaFile {
        range: "x6xxx",
        source: include_str!("../explanations/x6xxx.md"),
    },
    AreaFile {
        range: "x7xxx",
        source: include_str!("../explanations/x7xxx.md"),
    },
    AreaFile {
        range: "x8xxx",
        source: include_str!("../explanations/x8xxx.md"),
    },
];

/// The explanation of a code such as `"E2001"`, if one is written.
pub fn explanation(code: &str) -> Option<&'static Explanation> {
    all().get(&code.to_ascii_uppercase())
}

/// Every area with its range and short name (`("x2xxx", "types")`), in order.
pub fn areas() -> Vec<(&'static str, &'static str)> {
    FILES
        .iter()
        .map(|f| (f.range, short_name(f.source)))
        .collect()
}

fn all() -> &'static HashMap<String, Explanation> {
    static ALL: OnceLock<HashMap<String, Explanation>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut out = HashMap::new();
        for file in FILES {
            for (code, explanation) in parse(file.source) {
                out.insert(code, explanation);
            }
        }
        out
    })
}

/// The heading `# x2xxx · types` names the area `types`.
fn short_name(source: &'static str) -> &'static str {
    source
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .and_then(|h| h.split('·').nth(1))
        .map(str::trim)
        .unwrap_or("")
}

/// The `## CODE` sections of an area's file.
fn parse(source: &'static str) -> Vec<(String, Explanation)> {
    let area = short_name(source);
    let mut out: Vec<(String, Explanation)> = Vec::new();
    let mut lines = source.lines().peekable();
    // Consecutive blocks of the same verdict with a `title=` form one
    // example; any other block starts a new one.
    let mut joinable = false;
    while let Some(line) = lines.next() {
        if let Some(code) = line.strip_prefix("## ") {
            out.push((
                code.trim().to_string(),
                Explanation {
                    area,
                    text: String::new(),
                    examples: Vec::new(),
                    related: Vec::new(),
                },
            ));
            joinable = false;
            continue;
        }
        let Some((_, current)) = out.last_mut() else {
            continue;
        };
        if let Some(fence) = line.strip_prefix("```") {
            let mut words = fence.split_whitespace();
            let lang = words.next().unwrap_or("").to_string();
            let mut compiles = None;
            let mut title = None;
            for w in words {
                match w {
                    "fails" => compiles = Some(false),
                    "compiles" => compiles = Some(true),
                    _ => title = w.strip_prefix("title=").map(str::to_string).or(title),
                }
            }
            let mut text = String::new();
            for l in lines.by_ref() {
                if l.starts_with("```") {
                    break;
                }
                text.push_str(l);
                text.push('\n');
            }
            let Some(compiles) = compiles else {
                // A block that is neither: part of the explanation.
                current.text.push_str(&format!("```{fence}\n{text}```\n"));
                joinable = false;
                continue;
            };
            let path = title.clone().unwrap_or_else(|| {
                if text.contains("\"wowlua:test\"") || text.contains("\"wowlua:mock\"") {
                    "tests/main.test.wowlua".into()
                } else {
                    "src/main.wowlua".into()
                }
            });
            let file = ExampleFile { path, lang, text };
            match current.examples.last_mut() {
                Some(last) if joinable && title.is_some() && last.compiles == compiles => {
                    last.files.push(file)
                }
                _ => current.examples.push(Example {
                    compiles,
                    files: vec![file],
                }),
            }
            joinable = title.is_some();
            continue;
        }
        joinable = joinable && line.trim().is_empty();
        if let Some(related) = line.strip_prefix("Related:") {
            current.related = related
                .split(|c: char| c == ',' || c.is_whitespace())
                .map(|c| c.trim_matches('`'))
                .filter(|c| !c.is_empty())
                .map(str::to_string)
                .collect();
            continue;
        }
        current.text.push_str(line);
        current.text.push('\n');
    }
    for (_, e) in &mut out {
        e.text = e.text.trim().to_string();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_explanation_names_a_registered_code() {
        for (code, e) in all() {
            assert!(
                crate::lookup(code).is_some(),
                "{code} is explained but not registered"
            );
            assert!(!e.text.is_empty(), "{code} has no explanation text");
            for r in &e.related {
                // The warning forms of name and type errors in plain Lua files (W2001 for E2001) aren't listed.
                let downgraded =
                    r.starts_with('W') && crate::lookup(&format!("E{}", &r[1..])).is_some();
                assert!(
                    crate::lookup(r).is_some() || downgraded,
                    "{code} relates to unknown code {r}"
                );
            }
        }
    }

    #[test]
    fn sections_examples_and_related_codes_are_read() {
        let source = "# x2xxx · types\n\n## E2001\n\nA value's type does not fit.\n\n\
            ```wowlua fails\nlocal n: number = \"one\"\n```\n\n\
            ```wowlua compiles\nlocal n: number = 1\n```\n\n\
            Related: E2002, `E2003`\n\n## E2002\n\nUnknown type.\n\n\
            ```wowlua fails title=src/a.wowlua\nexport type A = B\n```\n\
            ```wowlua fails title=src/b.wowlua\nimport A from \"/a\"\n```\n";
        let parsed = parse(Box::leak(source.to_string().into_boxed_str()));
        assert_eq!(parsed.len(), 2);
        let (code, e) = &parsed[0];
        assert_eq!(code, "E2001");
        assert_eq!(e.area, "types");
        assert_eq!(e.text, "A value's type does not fit.");
        assert_eq!(e.examples.len(), 2);
        assert!(!e.examples[0].compiles && e.examples[1].compiles);
        assert_eq!(e.examples[0].files[0].path, "src/main.wowlua");
        assert_eq!(e.related, ["E2002", "E2003"]);
        let (_, e) = &parsed[1];
        assert_eq!(
            e.examples.len(),
            1,
            "titled blocks of one verdict are one example"
        );
        assert_eq!(e.examples[0].files.len(), 2);
    }
}
