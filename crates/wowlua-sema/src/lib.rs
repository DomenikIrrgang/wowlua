//! Semantic analysis (spec compiler §13.6): the module structure of a
//! package (exports, imports, specifier resolution), constant evaluation
//! across modules, and the dependency order of files.

pub mod constant;
pub mod order;

use constant::Const;
use indexmap::IndexMap;
use std::collections::HashMap;
use wowlua_diagnostics::Diagnostic;
use wowlua_span::Span;
use wowlua_syntax::Parse;
use wowlua_syntax::ast::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Wowlua,
    Lua,
    Xml,
    /// `.d.wowlua` (types only).
    Declaration,
}

/// One file of the package, as the analysis sees it.
pub struct FileInput<'a> {
    /// Path relative to the source root, `/` separators.
    pub src_rel: String,
    pub kind: FileKind,
    pub parse: Option<&'a Parse>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Export {
    Value,
    Function,
    /// `export const`; the value when it folds.
    Const(Option<Const>),
    Type,
}

/// What an import statement refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A `.wowlua` module (input index).
    Module(usize),
    /// The compiler-known `wowlua:meta`.
    Meta,
    /// A plain Lua file (input index); with a declaration file when it
    /// registers a typed module.
    Lua { file: usize, declared: bool },
    /// An XML file (load order only).
    Xml(usize),
}

#[derive(Clone, Debug)]
pub struct Import {
    /// Start of the import statement (identifies it during lowering).
    pub start: u32,
    pub target: Target,
    pub type_only: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ModuleInfo {
    /// Module ID: the path below the source root without extension.
    pub id: String,
    /// The file has an `import` or `export` (otherwise it is a script).
    pub is_module: bool,
    pub exports: IndexMap<String, Export>,
    pub imports: Vec<Import>,
}

#[derive(Debug, Default)]
pub struct Analysis {
    /// Per input file (`.wowlua` files only).
    pub modules: Vec<Option<ModuleInfo>>,
    /// `(a, b)`: input `a` must load before input `b` (from imports).
    pub edges: Vec<(usize, usize)>,
    pub diagnostics: Vec<Diagnostic>,
}

/// The module ID of a source path: `core/cache.wowlua` → `core/cache`.
pub fn module_id(src_rel: &str) -> String {
    for ext in [".d.wowlua", ".wowlua", ".lua", ".xml"] {
        if let Some(stem) = src_rel.strip_suffix(ext) {
            return stem.to_string();
        }
    }
    src_rel.to_string()
}

/// Analyzes the modules of a package.
pub fn analyze(files: &[FileInput]) -> Analysis {
    let mut analysis = Analysis {
        modules: vec![None; files.len()],
        ..Analysis::default()
    };
    let exact: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.src_rel.as_str(), i))
        .collect();
    let folded: HashMap<String, &str> = files
        .iter()
        .map(|f| (f.src_rel.to_ascii_lowercase(), f.src_rel.as_str()))
        .collect();

    // Exports and imports of every module.
    for (i, f) in files.iter().enumerate() {
        if f.kind != FileKind::Wowlua {
            continue;
        }
        let Some(parse) = f.parse else { continue };
        let mut info = ModuleInfo {
            id: module_id(&f.src_rel),
            ..ModuleInfo::default()
        };
        let mut seen_other = false;
        for s in &parse.chunk.block.stmts {
            match &s.kind {
                StmtKind::Import(import) => {
                    info.is_module = true;
                    if seen_other {
                        analysis.diagnostics.push(
                            Diagnostic::error(
                                "E1011",
                                "imports must come before every other statement",
                            )
                            .with_label(s.span, "this import follows other code"),
                        );
                    }
                    let resolved = resolve(&import.from, &f.src_rel, &exact, &folded, files);
                    match resolved {
                        Ok(target) => {
                            check_names(import, &target, files, &mut analysis.diagnostics);
                            if !import.type_only
                                && let Some(dep) = target_file(&target)
                            {
                                analysis.edges.push((dep, i));
                            }
                            info.imports.push(Import {
                                start: s.span.start,
                                target,
                                type_only: import.type_only,
                            });
                        }
                        Err(d) => analysis
                            .diagnostics
                            .push(d.with_label(string_span(&import.from, s), "here")),
                    }
                }
                StmtKind::Export(inner) => {
                    info.is_module = true;
                    seen_other = true;
                    match &inner.kind {
                        StmtKind::Local { names, .. } => {
                            for n in names {
                                info.exports.insert(n.name.text.to_string(), Export::Value);
                            }
                        }
                        StmtKind::Function { name, .. } => {
                            info.exports
                                .insert(name.path[0].text.to_string(), Export::Function);
                        }
                        StmtKind::Const { name, .. } => {
                            info.exports
                                .insert(name.text.to_string(), Export::Const(None));
                        }
                        StmtKind::TypeAlias(t) => {
                            info.exports.insert(t.name.text.to_string(), Export::Type);
                        }
                        StmtKind::Interface(t) => {
                            info.exports.insert(t.name.text.to_string(), Export::Type);
                        }
                        StmtKind::Class(c) => {
                            info.exports.insert(c.name.text.to_string(), Export::Value);
                        }
                        StmtKind::Enum(e) => {
                            info.exports.insert(e.name.text.to_string(), Export::Value);
                        }
                        _ => {}
                    }
                }
                _ => seen_other = true,
            }
        }
        if info.is_module {
            for s in &parse.chunk.block.stmts {
                if matches!(s.kind, StmtKind::Return(_)) {
                    analysis.diagnostics.push(
                        Diagnostic::error("E1032", "a module cannot `return` at its top level")
                            .with_label(s.span, "exports after this would never be assigned"),
                    );
                }
            }
        }
        analysis.modules[i] = Some(info);
    }

    // Exported constants, across modules (repeated until nothing changes).
    for _ in 0..=files.len() {
        let mut changed = false;
        for (i, file) in files.iter().enumerate() {
            let Some(parse) = file.parse else {
                continue;
            };
            let Some(info) = &analysis.modules[i] else {
                continue;
            };
            let mut env: HashMap<String, Const> = imported_consts(info, &analysis.modules, parse);
            let mut updates = Vec::new();
            for s in &parse.chunk.block.stmts {
                let (name, value, exported) = match &s.kind {
                    StmtKind::Const { name, value, .. } => (name, value, false),
                    StmtKind::Export(inner) => match &inner.kind {
                        StmtKind::Const { name, value, .. } => (name, value, true),
                        _ => continue,
                    },
                    _ => continue,
                };
                let Some(value) = value else { continue };
                if let Some(c) = constant::eval(value, &|n| env.get(n).cloned()) {
                    env.insert(name.text.to_string(), c.clone());
                    if exported {
                        updates.push((name.text.to_string(), c));
                    }
                }
            }
            let info = analysis.modules[i].as_mut().expect("checked");
            for (name, c) in updates {
                let slot = info.exports.get_mut(&name).expect("collected");
                if *slot != Export::Const(Some(c.clone())) {
                    *slot = Export::Const(Some(c));
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    analysis
}

/// The folded constants a module imports, by local name.
fn imported_consts(
    info: &ModuleInfo,
    modules: &[Option<ModuleInfo>],
    parse: &Parse,
) -> HashMap<String, Const> {
    let mut env = HashMap::new();
    for s in &parse.chunk.block.stmts {
        let StmtKind::Import(import) = &s.kind else {
            continue;
        };
        let Some(resolved) = info.imports.iter().find(|i| i.start == s.span.start) else {
            continue;
        };
        let Target::Module(m) = resolved.target else {
            continue;
        };
        let Some(target) = &modules[m] else { continue };
        if let ImportClause::Names(specs) = &import.clause {
            for spec in specs {
                if let Some(Export::Const(Some(c))) = target.exports.get(spec.name.text.as_ref()) {
                    let local = spec.alias.as_ref().unwrap_or(&spec.name);
                    env.insert(local.text.to_string(), c.clone());
                }
            }
        }
    }
    env
}

fn target_file(target: &Target) -> Option<usize> {
    match target {
        Target::Module(i) | Target::Xml(i) | Target::Lua { file: i, .. } => Some(*i),
        Target::Meta => None,
    }
}

fn string_span(lit: &StringLit, stmt: &Stmt) -> Span {
    // The specifier is the last token of the statement.
    let len = lit.raw.len() as u32;
    Span {
        file: stmt.span.file,
        start: stmt.span.end.saturating_sub(len),
        end: stmt.span.end,
    }
}

fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// Resolves a module specifier (spec §5.3).
fn resolve(
    spec: &StringLit,
    importer: &str,
    exact: &HashMap<&str, usize>,
    folded: &HashMap<String, &str>,
    files: &[FileInput],
) -> Result<Target, Diagnostic> {
    let text = String::from_utf8_lossy(&spec.value).into_owned();
    if text == "wowlua:meta" {
        return Ok(Target::Meta);
    }
    if text.starts_with("wowlua:") {
        return Err(Diagnostic::error(
            "E1015",
            format!(
                "standard-library module `{text}` is not available yet (the standard library arrives with milestone M3)"
            ),
        ));
    }
    let base = if let Some(rest) = text.strip_prefix('/') {
        rest.to_string()
    } else if text.starts_with("./") || text.starts_with("../") {
        let dir = importer.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        format!("{dir}/{text}")
    } else {
        return Err(Diagnostic::error(
            "E1015",
            format!(
                "package `{text}` not found (dependencies arrive with the package manager, milestone M6)"
            ),
        ));
    };
    let Some(path) = normalize(&base) else {
        return Err(Diagnostic::error(
            "E1015",
            format!("`{text}` leaves the source root"),
        ));
    };
    let has_ext = [".xml", ".lua", ".wowlua"]
        .iter()
        .any(|e| path.ends_with(e));
    let candidates: Vec<String> = if has_ext {
        vec![path.clone()]
    } else {
        vec![
            format!("{path}.wowlua"),
            format!("{path}.d.wowlua"),
            format!("{path}.lua"),
            format!("{path}/init.wowlua"),
        ]
    };
    for candidate in &candidates {
        if let Some(&i) = exact.get(candidate.as_str()) {
            return Ok(match files[i].kind {
                FileKind::Wowlua => Target::Module(i),
                FileKind::Xml => Target::Xml(i),
                FileKind::Lua => Target::Lua {
                    file: i,
                    declared: exact.contains_key(format!("{path}.d.wowlua").as_str()),
                },
                FileKind::Declaration => {
                    // Types come from the declaration; a sibling `.lua`
                    // provides the values.
                    match exact.get(format!("{path}.lua").as_str()) {
                        Some(&lua) => Target::Lua {
                            file: lua,
                            declared: true,
                        },
                        None => continue,
                    }
                }
            });
        }
    }
    for candidate in &candidates {
        if let Some(actual) = folded.get(&candidate.to_ascii_lowercase()) {
            return Err(Diagnostic::error(
                "E1013",
                format!("`{text}` differs in case from the file `{actual}`"),
            ));
        }
    }
    Err(Diagnostic::error(
        "E1015",
        format!("module `{text}` not found"),
    ))
}

/// Named imports must exist in the target (E1012).
fn check_names(
    import: &wowlua_syntax::ast::Import,
    target: &Target,
    files: &[FileInput],
    out: &mut Vec<Diagnostic>,
) {
    let ImportClause::Names(specs) = &import.clause else {
        if let (
            ImportClause::Namespace(name),
            Target::Xml(_)
            | Target::Lua {
                declared: false, ..
            },
        ) = (&import.clause, target)
        {
            out.push(
                Diagnostic::error(
                    "E1012",
                    "this file has no exports; load it with `import _ from …`",
                )
                .with_label(name.span, "here"),
            );
        }
        return;
    };
    let available: Option<Vec<String>> = match target {
        Target::Module(i) => files[*i].parse.map(exported_names),
        Target::Lua {
            declared: true,
            file,
        } => {
            // Names from the sibling declaration file's `declare module`.
            let decl = module_id(&files[*file].src_rel) + ".d.wowlua";
            files
                .iter()
                .find(|f| f.src_rel == decl)
                .and_then(|f| f.parse)
                .map(declared_names)
        }
        Target::Lua {
            declared: false, ..
        }
        | Target::Xml(_) => Some(Vec::new()),
        Target::Meta => None,
    };
    let Some(available) = available else { return };
    for spec in specs {
        if !available.iter().any(|n| n == spec.name.text.as_ref()) {
            let message = match target {
                Target::Xml(_)
                | Target::Lua {
                    declared: false, ..
                } => "this file has no exports; load it with `import _ from …`".to_string(),
                _ => format!("module has no export named `{}`", spec.name.text),
            };
            out.push(
                Diagnostic::error("E1012", message).with_label(spec.name.span, "not exported"),
            );
        }
    }
}

fn exported_names(parse: &Parse) -> Vec<String> {
    let mut out = Vec::new();
    for s in &parse.chunk.block.stmts {
        let StmtKind::Export(inner) = &s.kind else {
            continue;
        };
        match &inner.kind {
            StmtKind::Local { names, .. } => {
                out.extend(names.iter().map(|n| n.name.text.to_string()))
            }
            StmtKind::Function { name, .. } => out.push(name.path[0].text.to_string()),
            StmtKind::Const { name, .. } => out.push(name.text.to_string()),
            StmtKind::TypeAlias(t) => out.push(t.name.text.to_string()),
            StmtKind::Interface(t) => out.push(t.name.text.to_string()),
            StmtKind::Class(c) => out.push(c.name.text.to_string()),
            StmtKind::Enum(e) => out.push(e.name.text.to_string()),
            _ => {}
        }
    }
    out
}

fn declared_names(parse: &Parse) -> Vec<String> {
    let mut out = Vec::new();
    for s in &parse.chunk.block.stmts {
        let StmtKind::Declare(d) = &s.kind else {
            continue;
        };
        let Declare::Module { members, .. } = d.as_ref() else {
            continue;
        };
        for m in members {
            let name = match &m.kind {
                NsMemberKind::Function { name, .. }
                | NsMemberKind::Value { name, .. }
                | NsMemberKind::Namespace { name, .. }
                | NsMemberKind::Class { name, .. } => name,
                NsMemberKind::Interface(i) => &i.name,
                NsMemberKind::TypeAlias(t) => &t.name,
                NsMemberKind::Enum(e) => &e.name,
            };
            out.push(name.text.to_string());
        }
    }
    out
}
