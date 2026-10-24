//! Go to definition (spec §16.2): the names a checked package uses, each
//! with the span of what it refers to. The checker records locals, members
//! and types by span as it meets them, imports as the export they name and
//! globals by name; this module resolves those last two from the syntax of
//! the package's files once all of them are checked.

use super::env::DefTarget;
use crate::{Analysis, FileInput, FileKind, Target};
use rustc_hash::{FxHashMap, FxHashSet};
use wowlua_span::Span;
use wowlua_syntax::view::*;

/// Imported bindings' targets, by local name.
pub(crate) type ImportBindings = FxHashMap<Box<str>, DefTarget>;

/// The targets of file `i`'s imported bindings, by local name, and the
/// names its import statements themselves write (the imported names and
/// a namespace import's name), with their targets.
pub(crate) fn import_targets(
    files: &[FileInput],
    analysis: &Analysis,
    i: usize,
) -> (ImportBindings, Vec<(Span, DefTarget)>) {
    let mut bindings = FxHashMap::default();
    let mut written = Vec::new();
    let (Some(parse), Some(info)) = (files[i].parse, &analysis.modules[i]) else {
        return (bindings, written);
    };
    for s in parse.view().block().stmts() {
        let StmtKind::Import(import) = s.kind() else {
            continue;
        };
        let Some(Target::Module(m)) = info
            .imports
            .iter()
            .find(|r| r.start == s.span().start)
            .map(|r| r.target.clone())
        else {
            continue;
        };
        match import.clause() {
            ImportClause::Names(specs) => {
                for spec in specs {
                    let target = DefTarget::Export(m, spec.name.text().into());
                    let local = spec.alias.as_ref().unwrap_or(&spec.name);
                    bindings.insert(local.text().into(), target.clone());
                    written.push((spec.name.span(), target.clone()));
                    if let Some(alias) = spec.alias {
                        written.push((alias.span(), target));
                    }
                }
            }
            ImportClause::Namespace(name) => {
                bindings.insert(name.text().into(), DefTarget::Module(m));
                written.push((name.span(), DefTarget::Module(m)));
            }
            ImportClause::SideEffect => {}
        }
    }
    (bindings, written)
}

/// The definitions of `found`, as (name, what it refers to), for names in
/// the package's own files (not the compiler's, the game's or a
/// dependency's); a global no source defines and an export no module has
/// are left out.
pub(crate) fn resolve(
    files: &[FileInput],
    analysis: &Analysis,
    mut found: Vec<(Span, DefTarget)>,
) -> Vec<(Span, Span)> {
    let own: FxHashSet<wowlua_span::FileId> = files
        .iter()
        .filter(|f| {
            !crate::is_compiler_known(&f.src_rel)
                && !f.src_rel.starts_with('<')
                && crate::package_of(&f.src_rel).is_none()
        })
        .filter_map(|f| f.parse)
        .map(|p| p.view().block().span().file)
        .collect();
    found.retain(|(at, _)| own.contains(&at.file));
    let wanted: FxHashSet<&str> = found
        .iter()
        .filter_map(|(_, t)| match t {
            DefTarget::Global(name) => Some(name.as_ref()),
            _ => None,
        })
        .collect();
    let globals = global_spans(files, analysis, &wanted);
    let mut exports: FxHashMap<usize, FxHashMap<&str, Span>> = FxHashMap::default();
    let mut out: Vec<(Span, Span)> = Vec::with_capacity(found.len());
    let mut seen: FxHashSet<(Span, Span)> = FxHashSet::default();
    for (at, target) in &found {
        let to = match target {
            DefTarget::At(span) => Some(*span),
            DefTarget::Global(name) => globals.get(name.as_ref()).copied(),
            DefTarget::Module(m) => module_start(files, *m),
            DefTarget::Export(m, name) => {
                let spans = exports.entry(*m).or_insert_with(|| export_spans(files, *m));
                spans
                    .get(name.as_ref())
                    .copied()
                    .or_else(|| module_start(files, *m))
            }
        };
        if let Some(to) = to
            && seen.insert((*at, to))
        {
            out.push((*at, to));
        }
    }
    out
}

/// The start of input `m`'s file.
fn module_start(files: &[FileInput], m: usize) -> Option<Span> {
    let parse = files.get(m)?.parse?;
    Some(Span::file_start(parse.view().block().span().file))
}

/// The names module `m` exports, where it declares them.
fn export_spans<'a>(files: &'a [FileInput], m: usize) -> FxHashMap<&'a str, Span> {
    let mut out = FxHashMap::default();
    let Some(parse) = files.get(m).and_then(|f| f.parse) else {
        return out;
    };
    for s in parse.view().block().stmts() {
        if let StmtKind::Export(inner) = s.kind() {
            for name in declared_names(inner.kind()) {
                out.entry(name.text()).or_insert(name.span());
            }
        }
    }
    out
}

/// The names a statement declares at its level.
fn declared_names(kind: StmtKind<'_>) -> Vec<Name<'_>> {
    match kind {
        StmtKind::Function { name, .. } if name.method().is_none() && name.path().len() == 1 => {
            vec![name.path().at(0)]
        }
        StmtKind::LocalFunction { name, .. } | StmtKind::Const { name, .. } => vec![name],
        StmtKind::Local { names, .. } => names.iter().map(|n| n.name).collect(),
        StmtKind::Class(c) => vec![c.name()],
        StmtKind::Interface(i) => vec![i.name()],
        StmtKind::Enum(e) => vec![e.name()],
        StmtKind::TypeAlias(a) => vec![a.name()],
        StmtKind::Declare(d) => match d.kind() {
            DeclareKind::Function { name, .. }
                if name.method().is_none() && name.path().len() == 1 =>
            {
                vec![name.path().at(0)]
            }
            DeclareKind::Global { name, .. }
            | DeclareKind::Namespace { name, .. }
            | DeclareKind::Class { name, .. } => vec![name],
            DeclareKind::Interface(i) => vec![i.name()],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Where the globals of `wanted` are defined: a script's top-level
/// `function name()` or `name = …` first, then a declaration.
fn global_spans<'a>(
    files: &[FileInput],
    analysis: &Analysis,
    wanted: &FxHashSet<&'a str>,
) -> FxHashMap<&'a str, Span> {
    let mut out: FxHashMap<&'a str, Span> = FxHashMap::default();
    if wanted.is_empty() {
        return out;
    }
    let keep = |out: &mut FxHashMap<&'a str, Span>, name: Name<'_>| {
        if let Some(&w) = wanted.get(name.text()) {
            out.entry(w).or_insert(name.span());
        }
    };
    // Scripts' own definitions (a module's top-level functions are its
    // locals).
    for (i, f) in files.iter().enumerate() {
        let script = match f.kind {
            FileKind::Lua => true,
            FileKind::Wowlua => !analysis
                .modules
                .get(i)
                .and_then(Option::as_ref)
                .is_some_and(|m| m.is_module),
            _ => false,
        };
        let Some(parse) = f.parse.filter(|_| script) else {
            continue;
        };
        for s in parse.view().block().stmts() {
            match s.kind() {
                StmtKind::Function { name, .. }
                    if name.method().is_none() && name.path().len() == 1 =>
                {
                    keep(&mut out, name.path().at(0));
                }
                StmtKind::Assign { targets, .. } => {
                    for t in targets.iter() {
                        if let ExprKind::Name(n) = t.kind() {
                            keep(&mut out, n);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // Declarations: the package's, the standard library's and the game's.
    for f in files {
        let Some(parse) = f.parse else { continue };
        for s in parse.view().block().stmts() {
            let kind = match s.kind() {
                StmtKind::Export(inner) => inner.kind(),
                k => k,
            };
            if matches!(kind, StmtKind::Declare(_)) || f.kind == FileKind::Declaration {
                for name in declared_names(kind) {
                    keep(&mut out, name);
                }
            }
        }
    }
    out
}
