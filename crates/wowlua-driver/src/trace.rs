//! `wowlua trace` (spec tooling §16.4): error messages and stacks from the
//! game, mapped back to source locations. Locations in files with a source
//! map (bundles, statement-mode output) map through the map of the build
//! (by build id: `--build`, or the id load errors of bundles name; else the
//! newest build in `dist/`); line-preserved files map to the same line of
//! their source. The enclosing function is found in the source, and renamed
//! locals get their original names.

use crate::Project;
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;
use wowlua_luair::sourcemap::SourceMap;
use wowlua_syntax::ast::*;

/// The decoded text: every input line, each location followed by
/// `  → src/x.wowlua:12 in name`.
pub fn decode(project: &Project, input: &str, build: Option<&str>) -> String {
    let addon = project.addon_name();
    let root = project.root();
    let named_build = Regex::new(r"build ([0-9a-f]{6,})\b").expect("a valid pattern");
    let build = build
        .map(str::to_string)
        .or_else(|| named_build.captures(input).map(|c| c[1].to_string()));
    let builds = map_dirs(project, build.as_deref());
    let mut map_cache: HashMap<String, Option<(SourceMap, Option<String>)>> = HashMap::new();
    let local_name =
        Regex::new(r"(?:local|upvalue) '([A-Za-z_][A-Za-z0-9_]*)'").expect("a valid pattern");
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
            let mapped = map_cache
                .entry(path.to_ascii_lowercase())
                .or_insert_with(|| find_map(&builds, &path))
                .as_ref()
                .and_then(|(map, generated)| {
                    let seg = map
                        .segments
                        .iter()
                        .filter(|s| s.out_line == number)
                        .min_by_key(|s| s.out_col)?;
                    let source = map.sources.get(seg.source as usize)?.clone();
                    // A renamed local the message names.
                    let original = local_name.captures(line).and_then(|c| {
                        let text = generated.as_ref()?.lines().nth(number as usize - 1)?;
                        let short = &c[1];
                        map.segments
                            .iter()
                            .filter(|s| s.out_line == number)
                            .find(|s| {
                                s.name.is_some()
                                    && text.get(s.out_col as usize..).is_some_and(|rest| {
                                        rest.starts_with(short)
                                            && !rest[short.len()..].starts_with(|ch: char| {
                                                ch.is_ascii_alphanumeric() || ch == '_'
                                            })
                                    })
                            })
                            .and_then(|s| map.names.get(s.name? as usize))
                            .map(|n| (short.to_string(), n.clone()))
                    });
                    Some((source, seg.src_line, original))
                });
            let (source, number, original) = match mapped {
                Some(found) => found,
                None => match outputs.get(&path.to_ascii_lowercase()) {
                    Some(source) => (source.clone(), number, None),
                    None => continue,
                },
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
            if let Some((short, long)) = original {
                out.push_str(&format!("   ('{short}' is '{long}')"));
            }
            out.push('\n');
        }
    }
    out
}

/// Where the maps of builds are: `(maps folder, AddOn folder)`, the build
/// `build` first (in `dist/`, then archived), else the newest builds.
fn map_dirs(project: &Project, build: Option<&str>) -> Vec<(PathBuf, Option<PathBuf>)> {
    let addon = project.addon_name();
    let dist = project.manifest.out_dir();
    let mut found: Vec<(
        PathBuf,
        Option<PathBuf>,
        Option<String>,
        std::time::SystemTime,
    )> = Vec::new();
    let mut parents: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&dist).into_iter().flatten().flatten() {
        if entry.file_name() == "targets" {
            parents.extend(
                std::fs::read_dir(entry.path())
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path()),
            );
        } else {
            parents.push(entry.path());
        }
    }
    for parent in parents {
        let maps = parent.join(".maps").join(&addon);
        let Ok(meta) = std::fs::metadata(&maps) else {
            continue;
        };
        let addon_dir = parent.join(&addon);
        let id = std::fs::read_to_string(addon_dir.join(format!("{addon}.toc")))
            .ok()
            .and_then(|toc| {
                toc.lines()
                    .find_map(|l| l.strip_prefix("## X-Wowlua-Build:"))
                    .and_then(|v| v.split_whitespace().last().map(str::to_string))
            });
        let time = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        found.push((maps, Some(addon_dir), id, time));
    }
    if let Some(home) = crate::install::wowlua_home() {
        for entry in std::fs::read_dir(home.join("maps"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let maps = entry.path().join(&addon);
            if maps.is_dir() {
                let id = entry.file_name().to_string_lossy().into_owned();
                found.push((maps, None, Some(id), std::time::UNIX_EPOCH));
            }
        }
    }
    match build {
        Some(id) => found
            .into_iter()
            .filter(|(_, _, b, _)| {
                b.as_deref()
                    .is_some_and(|b| b.starts_with(id) || id.starts_with(b))
            })
            .map(|(m, a, _, _)| (m, a))
            .collect(),
        None => {
            // Archived maps need a build id.
            found.retain(|(_, a, _, _)| a.is_some());
            found.sort_by_key(|f| std::cmp::Reverse(f.3));
            found.into_iter().map(|(m, a, _, _)| (m, a)).collect()
        }
    }
}

/// The map of output file `path` in the first build that has one, and the
/// file's text when the build is in `dist/`.
fn find_map(
    builds: &[(PathBuf, Option<PathBuf>)],
    path: &str,
) -> Option<(SourceMap, Option<String>)> {
    builds.iter().find_map(|(maps, addon_dir)| {
        let text = std::fs::read_to_string(maps.join(format!("{path}.map"))).ok()?;
        let map = SourceMap::parse(&text)?;
        let generated = addon_dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join(path)).ok());
        Some((map, generated))
    })
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
    wowlua_syntax::visit::walk_stmts(
        wowlua_syntax::view::Block::of(&parse.chunk.block),
        &mut |s| match &s.ast().kind {
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
        },
    );
    out
}
