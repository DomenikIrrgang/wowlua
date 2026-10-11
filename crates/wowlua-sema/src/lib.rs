//! Semantic analysis (spec compiler §13.6): the module structure of a
//! package (exports, imports, specifier resolution), constant evaluation
//! across modules, and the dependency order of files.

pub mod check;
pub mod classes;
pub mod constant;
pub mod loadorder;
pub mod meta;
pub mod order;
pub mod usage;

use constant::Const;
use indexmap::IndexMap;
use meta::{Meta, MetaKind};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
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
    /// The text of an XML file, for the load order.
    pub xml: Option<(&'a str, wowlua_span::FileId)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Export {
    Value,
    Function,
    /// `export const`; the value when it folds.
    Const(Option<Const>),
    Type,
    Class(Arc<classes::ClassInfo>),
    Enum(Arc<classes::EnumInfo>),
}

/// `@deprecated` and `@debugOnly` on an export (spec §4.3).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportAttrs {
    /// `@deprecated(msg?)`: the message, if any.
    pub deprecated: Option<Option<String>>,
    pub debug_only: bool,
}

impl ExportAttrs {
    /// The core decorators of a statement that importers see.
    pub fn of(decorators: &[Decorator]) -> ExportAttrs {
        let mut attrs = ExportAttrs::default();
        for d in decorators {
            if d.path.len() != 1 {
                continue;
            }
            match d.path[0].text.as_ref() {
                "deprecated" => {
                    let message =
                        d.args
                            .as_ref()
                            .and_then(|a| match a.exprs().first().map(|e| &e.kind) {
                                Some(ExprKind::String(s)) => {
                                    Some(String::from_utf8_lossy(&s.value).into_owned())
                                }
                                _ => None,
                            });
                    attrs.deprecated = Some(message);
                }
                "debugOnly" => attrs.debug_only = true,
                _ => {}
            }
        }
        attrs
    }
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
    /// Names bound to `wowlua:meta` exports.
    pub meta_bindings: HashMap<String, MetaKind>,
    /// Core decorators of exports that importers must know about.
    pub export_attrs: HashMap<String, ExportAttrs>,
    /// Exports bound to compiler behavior: export name → `@intrinsic` id
    /// (from a standard-library module's declaration file).
    pub intrinsics: HashMap<String, String>,
    /// Top-level classes and enums of the file (exported or not).
    pub classes: IndexMap<String, Arc<classes::ClassInfo>>,
    pub enums: IndexMap<String, Arc<classes::EnumInfo>>,
}

#[derive(Debug, Default)]
pub struct Analysis {
    /// Per input file (`.wowlua` files only).
    pub modules: Vec<Option<ModuleInfo>>,
    /// `(a, b)`: input `a` must load before input `b` (from imports).
    pub edges: Vec<(usize, usize)>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Standard-library sources live under this prefix (`@wowlua/events.wowlua`).
pub const STDLIB_PREFIX: &str = "@wowlua/";

/// A compiler-known file: a standard-library module or declaration.
pub fn is_compiler_known(src_rel: &str) -> bool {
    src_rel.starts_with(STDLIB_PREFIX) || src_rel.starts_with("<wowlua>/")
}

/// The module ID of a source path: `core/cache.wowlua` → `core/cache`;
/// standard-library modules are `wowlua:<name>`.
pub fn module_id(src_rel: &str) -> String {
    if let Some(name) = src_rel
        .strip_prefix(STDLIB_PREFIX)
        .and_then(|n| n.strip_suffix(".wowlua"))
    {
        return format!("wowlua:{name}");
    }
    for ext in [".d.wowlua", ".wowlua", ".lua", ".xml"] {
        if let Some(stem) = src_rel.strip_suffix(ext) {
            return stem.to_string();
        }
    }
    src_rel.to_string()
}

/// Analyzes the modules of a package, for one build configuration and
/// flavor (`meta`: `@if` on imports changes a flavor's import graph).
pub fn analyze(files: &[FileInput], meta: Option<&Meta>) -> Analysis {
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
                    if import_removed(s, &info, meta, &mut analysis.diagnostics) {
                        continue;
                    }
                    let resolved = resolve(&import.from, &f.src_rel, &exact, &folded, files);
                    match resolved {
                        Ok(target) => {
                            check_names(import, &target, files, &mut analysis.diagnostics);
                            if target == Target::Meta {
                                bind_meta(import, &mut info.meta_bindings);
                            }
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
                    let attrs = ExportAttrs::of(&s.decorators);
                    if attrs != ExportAttrs::default() {
                        for name in exported_names_of(inner) {
                            info.export_attrs.insert(name, attrs.clone());
                        }
                    }
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
                let kinds = &info.meta_bindings;
                let folded = match meta {
                    Some(meta) => {
                        meta::static_value(value, meta, &|n| kinds.get(n).copied(), &|n| {
                            env.get(n).cloned()
                        })
                    }
                    None => constant::eval(value, &|n| env.get(n).cloned()),
                };
                if let Some(c) = folded {
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
    classes_and_enums(files, &mut analysis);
    intrinsics(files, &mut analysis);
    analysis
}

/// `@intrinsic("id")` declarations: in a standard-library module's
/// declaration file they bind the module's exports to compiler behavior;
/// anywhere else they are E3033.
fn intrinsics(files: &[FileInput], analysis: &mut Analysis) {
    let index: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.src_rel.as_str(), i))
        .collect();
    for f in files.iter().filter(|f| f.kind == FileKind::Declaration) {
        let Some(parse) = f.parse else { continue };
        let module = f
            .src_rel
            .strip_suffix(".d.wowlua")
            .filter(|_| f.src_rel.starts_with(STDLIB_PREFIX))
            .and_then(|stem| index.get(format!("{stem}.wowlua").as_str()).copied());
        for s in &parse.chunk.block.stmts {
            for d in s.decorators.iter().filter(|d| d.is_named("intrinsic")) {
                let Some(m) = module else {
                    if !is_compiler_known(&f.src_rel) {
                        analysis.diagnostics.push(
                            Diagnostic::error(
                                "E3033",
                                "`@intrinsic` is only for compiler-known modules (the standard library)",
                            )
                            .with_label(d.span, "here"),
                        );
                    }
                    continue;
                };
                let id = match d
                    .args
                    .as_ref()
                    .and_then(|a| a.exprs().first().map(|e| &e.kind))
                {
                    Some(ExprKind::String(lit)) => String::from_utf8_lossy(&lit.value).into_owned(),
                    _ => continue,
                };
                let name = match &s.kind {
                    StmtKind::Declare(decl) => match decl.as_ref() {
                        Declare::Function { name, .. } => name.path[0].text.to_string(),
                        Declare::Global { name, .. } => name.text.to_string(),
                        _ => continue,
                    },
                    _ => continue,
                };
                if let Some(info) = analysis.modules[m].as_mut() {
                    info.intrinsics.insert(name, id);
                }
            }
        }
    }
}

/// Classes and enums of every module (spec chapter 3): enum values, what
/// each class extends, widget classes, and the structural checks.
fn classes_and_enums(files: &[FileInput], analysis: &mut Analysis) {
    let declarations: Vec<&Parse> = files
        .iter()
        .filter(|f| f.kind == FileKind::Declaration)
        .filter_map(|f| f.parse)
        .collect();
    let widgets = classes::widget_decls(&declarations);
    // Enums first: their values fold with the module's constants.
    for (i, file) in files.iter().enumerate() {
        let (Some(parse), Some(info)) = (file.parse, &analysis.modules[i]) else {
            continue;
        };
        let env = module_consts(info, &analysis.modules, parse);
        let mut enums = IndexMap::new();
        for (e, _) in classes::declarations(parse).1 {
            let value = classes::enum_info(e, &|n| env.get(n).cloned(), &mut analysis.diagnostics);
            enums.insert(e.name.text.to_string(), Arc::new(value));
        }
        analysis.modules[i].as_mut().expect("checked").enums = enums;
    }
    let class_names: HashSet<(usize, String)> = files
        .iter()
        .enumerate()
        .filter(|(i, _)| analysis.modules[*i].is_some())
        .filter_map(|(i, f)| f.parse.map(|p| (i, p)))
        .flat_map(|(i, p)| {
            classes::declarations(p)
                .0
                .into_iter()
                .map(move |c| (i, c.name.text.to_string()))
        })
        .collect();
    let is_class = |m: usize, name: &str| class_names.contains(&(m, name.to_string()));
    // Raw infos with resolved bases.
    let mut raw: Vec<((usize, String), classes::ClassInfo)> = Vec::new();
    for (i, file) in files.iter().enumerate() {
        let (Some(parse), Some(info)) = (file.parse, &analysis.modules[i]) else {
            continue;
        };
        let (decls, _) = classes::declarations(parse);
        if decls.is_empty() {
            continue;
        }
        let local_classes: HashSet<String> =
            decls.iter().map(|c| c.name.text.to_string()).collect();
        let locals = top_level_locals(parse);
        let (named, namespaces) = import_bindings(info, parse);
        for c in decls {
            let base = match c.extends.as_ref().map(classes::type_path) {
                None => classes::Base::None,
                Some(None) => classes::Base::Unknown,
                Some(Some(path)) => classes::resolve_base(
                    &path,
                    i,
                    &local_classes,
                    &locals,
                    &named,
                    &namespaces,
                    &is_class,
                    &widgets,
                ),
            };
            raw.push((
                (i, c.name.text.to_string()),
                classes::ClassInfo {
                    name: c.name.text.to_string(),
                    is_abstract: c.is_abstract,
                    base,
                    widget: None,
                    members: classes::members(c),
                },
            ));
        }
    }
    // Widget classes: the chain reaches a native widget type.
    let bases: HashMap<(usize, String), classes::Base> = raw
        .iter()
        .map(|(k, v)| (k.clone(), v.base.clone()))
        .collect();
    for (_, info) in raw.iter_mut() {
        let mut base = info.base.clone();
        for _ in 0..64 {
            match base {
                classes::Base::Widget { widget, .. } => {
                    info.widget = Some(widget);
                    break;
                }
                classes::Base::Class { file, name } => match bases.get(&(file, name)) {
                    Some(b) => base = b.clone(),
                    None => break,
                },
                _ => break,
            }
        }
    }
    let table: classes::ClassTable = raw.into_iter().map(|(k, v)| (k, Arc::new(v))).collect();
    for (i, file) in files.iter().enumerate() {
        let (Some(parse), Some(_)) = (file.parse, &analysis.modules[i]) else {
            continue;
        };
        let mut module_classes = IndexMap::new();
        for c in classes::declarations(parse).0 {
            let info = table[&(i, c.name.text.to_string())].clone();
            classes::check_class(c, &info, &table, &mut analysis.diagnostics);
            module_classes.insert(c.name.text.to_string(), info);
        }
        let info = analysis.modules[i].as_mut().expect("checked");
        info.classes = module_classes;
        for (name, slot) in info.exports.iter_mut() {
            if let Some(c) = info.classes.get(name) {
                *slot = Export::Class(c.clone());
            } else if let Some(e) = info.enums.get(name) {
                *slot = Export::Enum(e.clone());
            }
        }
    }
}

/// The folded top-level constants of a module, imported ones included.
fn module_consts(
    info: &ModuleInfo,
    modules: &[Option<ModuleInfo>],
    parse: &Parse,
) -> HashMap<String, Const> {
    let mut env = imported_consts(info, modules, parse);
    for s in &parse.chunk.block.stmts {
        let StmtKind::Const {
            name,
            value: Some(value),
            ..
        } = (match &s.kind {
            StmtKind::Export(inner) => &inner.kind,
            k => k,
        })
        else {
            continue;
        };
        if let Some(c) = constant::eval(value, &|n| env.get(n).cloned()) {
            env.insert(name.text.to_string(), c);
        }
    }
    env
}

/// Names bound at the top level of a file other than classes.
fn top_level_locals(parse: &Parse) -> HashSet<String> {
    let mut out = HashSet::new();
    for s in &parse.chunk.block.stmts {
        let kind = match &s.kind {
            StmtKind::Export(inner) => &inner.kind,
            k => k,
        };
        match kind {
            StmtKind::Local { names, .. } => {
                out.extend(names.iter().map(|n| n.name.text.to_string()))
            }
            StmtKind::LocalFunction { name, .. } | StmtKind::Const { name, .. } => {
                out.insert(name.text.to_string());
            }
            StmtKind::LocalDestructure { fields, .. } => {
                out.extend(fields.iter().map(|f| f.name.text.to_string()))
            }
            StmtKind::Enum(e) => {
                out.insert(e.name.text.to_string());
            }
            _ => {}
        }
    }
    out
}

/// Imported bindings of a module: named (local → (module, export)) and
/// namespaces (local → module).
#[allow(clippy::type_complexity)]
fn import_bindings(
    info: &ModuleInfo,
    parse: &Parse,
) -> (HashMap<String, (usize, String)>, HashMap<String, usize>) {
    let mut named = HashMap::new();
    let mut namespaces = HashMap::new();
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
        match &import.clause {
            ImportClause::Names(specs) => {
                for spec in specs {
                    let local = spec.alias.as_ref().unwrap_or(&spec.name);
                    named.insert(local.text.to_string(), (m, spec.name.text.to_string()));
                }
            }
            ImportClause::Namespace(n) => {
                namespaces.insert(n.text.to_string(), m);
            }
            ImportClause::SideEffect => {}
        }
    }
    (named, namespaces)
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

/// Names bound by an import of `wowlua:meta`.
fn bind_meta(import: &wowlua_syntax::ast::Import, out: &mut HashMap<String, MetaKind>) {
    if let ImportClause::Names(specs) = &import.clause {
        for spec in specs {
            if let Some(kind) = MetaKind::from_export(&spec.name.text) {
                let local = spec.alias.as_ref().unwrap_or(&spec.name);
                out.insert(local.text.to_string(), kind);
            }
        }
    }
}

/// An `@if(condition)` on the import is false for this build (spec §6.4.3).
/// The condition may only use `wowlua:meta` paths and literals (E1040);
/// `flavor` is not static in shared mode (E1039).
fn import_removed(
    stmt: &Stmt,
    info: &ModuleInfo,
    meta: Option<&Meta>,
    out: &mut Vec<Diagnostic>,
) -> bool {
    let Some(meta) = meta else { return false };
    let kind_of = |n: &str| info.meta_bindings.get(n).copied();
    let mut removed = false;
    for d in &stmt.decorators {
        let Some(cond) = meta::if_condition(d) else {
            continue;
        };
        let Some(cond) = cond else {
            out.push(
                Diagnostic::error("E1040", "`@if` takes one condition").with_label(d.span, "here"),
            );
            continue;
        };
        match meta::static_value(cond, meta, &kind_of, &|_| None) {
            Some(c) => removed |= !c.truthy(),
            None if meta.flavor.is_none() && meta::uses_flavor(cond, &kind_of) => out.push(
                Diagnostic::error("E1039", "`flavor` is not a constant in shared flavor mode")
                    .with_label(cond.span, "use a runtime check instead"),
            ),
            None => out.push(
                Diagnostic::error(
                    "E1040",
                    "an `@if` on an import may only use `wowlua:meta` paths and literals",
                )
                .with_label(cond.span, "not static"),
            ),
        }
    }
    removed
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
    if let Some(name) = text.strip_prefix("wowlua:") {
        let path = format!("{STDLIB_PREFIX}{name}.wowlua");
        return match exact.get(path.as_str()) {
            Some(&i) => Ok(Target::Module(i)),
            None => Err(Diagnostic::error(
                "E1015",
                format!("unknown standard-library module `{text}`"),
            )),
        };
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
        Target::Meta => Some(
            ["addon", "build", "flavor", "config", "features", "asset"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        ),
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

/// The names an `export` statement declares.
fn exported_names_of(inner: &Stmt) -> Vec<String> {
    match &inner.kind {
        StmtKind::Local { names, .. } => names.iter().map(|n| n.name.text.to_string()).collect(),
        StmtKind::Function { name, .. } => vec![name.path[0].text.to_string()],
        StmtKind::Const { name, .. } => vec![name.text.to_string()],
        StmtKind::Class(c) => vec![c.name.text.to_string()],
        StmtKind::Enum(e) => vec![e.name.text.to_string()],
        _ => Vec::new(),
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
