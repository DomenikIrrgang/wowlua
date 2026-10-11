//! `wowlua trace` (spec tooling §16.4): error messages and stacks from the
//! game, mapped back to source locations. Builds are line-preserved until
//! bundling arrives (milestone M7), so a location maps to the same line of
//! its source file; the enclosing function is found in the source.

use crate::Project;
use regex::Regex;
use std::collections::HashMap;
use wowlua_syntax::ast::*;

/// The decoded text: every input line, each location followed by
/// `  → src/x.wowlua:12 in name`.
pub fn decode(project: &Project, input: &str) -> String {
    let addon = project.addon_name();
    let root = project.root();
    let src_dir = &project.manifest.manifest.build.src;
    // Output path (`src/core/cache.lua`) → source path relative to the root.
    let mut outputs: HashMap<String, String> = HashMap::new();
    for entry in walkdir::WalkDir::new(root.join(src_dir))
        .into_iter()
        .flatten()
    {
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let output = if let Some(stem) = rel.strip_suffix(".wowlua") {
            format!("{stem}.lua")
        } else if rel.ends_with(".lua") {
            rel.clone()
        } else {
            continue;
        };
        outputs.insert(output.to_ascii_lowercase(), rel);
    }
    let location = Regex::new(r#"([^\s"\[\]:<>]+?\.lua)"?\]?:(\d+)"#).expect("a valid pattern");
    let mut functions: HashMap<String, Vec<(u32, u32, String)>> = HashMap::new();
    let mut out = String::new();
    for line in input.lines() {
        out.push_str(line);
        out.push('\n');
        let mut seen = Vec::new();
        for cap in location.captures_iter(line) {
            let mut path = cap[1].replace('\\', "/");
            let number: u32 = cap[2].parse().unwrap_or(0);
            // `…/AddOns/<Addon>/src/a.lua` or `<Addon>/src/a.lua` →
            // `src/a.lua`; flavor directories go.
            let lower = path.to_ascii_lowercase();
            let marker = format!("addons/{}/", addon.to_ascii_lowercase());
            if let Some(i) = lower.find(&marker) {
                path = path[i + marker.len()..].to_string();
            } else if lower.starts_with(&format!("{}/", addon.to_ascii_lowercase())) {
                path = path[addon.len() + 1..].to_string();
            } else {
                continue;
            }
            if let Some(rest) = path.strip_prefix("wowlua/flavors/") {
                path = rest
                    .split_once('/')
                    .map(|(_, p)| p.to_string())
                    .unwrap_or(path);
            }
            let Some(source) = outputs.get(&path.to_ascii_lowercase()).cloned() else {
                continue;
            };
            if seen.contains(&(source.clone(), number)) {
                continue;
            }
            seen.push((source.clone(), number));
            let table = functions
                .entry(source.clone())
                .or_insert_with(|| function_ranges(&root.join(&source)));
            let name = table
                .iter()
                .filter(|(start, end, _)| *start <= number && number <= *end)
                .min_by_key(|(start, end, _)| end - start)
                .map(|(_, _, n)| n.clone());
            out.push_str(&format!("  → {source}:{number}"));
            if let Some(n) = name {
                out.push_str(&format!(" in {n}"));
            }
            out.push('\n');
        }
    }
    out
}

/// The line ranges of the named functions of a source file.
fn function_ranges(path: &std::path::Path) -> Vec<(u32, u32, String)> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let mut db = wowlua_span::SourceDb::new();
    let file = db.add("trace", String::from_utf8_lossy(&bytes).into_owned());
    let parse = wowlua_syntax::parse(&bytes, file);
    let lexed = &parse.lexed;
    let mut out = Vec::new();
    let mut add = |f: &FuncBody, name: String| {
        out.push((lexed.line_of(f.span.start), lexed.line_of(f.span.end), name));
    };
    fn path_name(e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Name(n) => Some(n.text.to_string()),
            ExprKind::Field { obj, name } => Some(format!("{}.{}", path_name(obj)?, name.text)),
            _ => None,
        }
    }
    wowlua_syntax::visit::walk_stmts(&parse.chunk.block, &mut |s| match &s.kind {
        StmtKind::Export(inner) => {
            if let StmtKind::Function { name, func } = &inner.kind {
                add(
                    func,
                    name.path
                        .iter()
                        .map(|p| p.text.as_ref())
                        .collect::<Vec<_>>()
                        .join("."),
                );
            }
        }
        StmtKind::Function { name, func } => {
            let mut n = name
                .path
                .iter()
                .map(|p| p.text.as_ref())
                .collect::<Vec<_>>()
                .join(".");
            if let Some(m) = &name.method {
                n = format!("{n}:{}", m.text);
            }
            add(func, n);
        }
        StmtKind::LocalFunction { name, func } => add(func, name.text.to_string()),
        StmtKind::Local { names, values } => {
            for (n, v) in names.iter().zip(values) {
                if let ExprKind::Function(f) = &v.kind {
                    add(f, n.name.text.to_string());
                }
            }
        }
        StmtKind::Assign { targets, values } => {
            for (t, v) in targets.iter().zip(values) {
                if let (ExprKind::Function(f), Some(n)) = (&v.kind, path_name(t)) {
                    add(f, n);
                }
            }
        }
        StmtKind::Class(c) => {
            for m in &c.members {
                match &m.kind {
                    ClassMemberKind::Method { name, func, .. } => {
                        let sep = if m.modifiers.contains(&Modifier::Static) {
                            "."
                        } else {
                            ":"
                        };
                        add(func, format!("{}{sep}{}", c.name.text, name.text));
                    }
                    ClassMemberKind::Constructor { func, .. } => {
                        add(func, format!("{}.new", c.name.text))
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    });
    out
}
