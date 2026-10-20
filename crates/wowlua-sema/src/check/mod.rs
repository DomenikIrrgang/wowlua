//! The type checker (spec chapter 2, compiler §13.6): declarations, type
//! resolution, assignability and the bidirectional checking of function
//! bodies with flow-sensitive narrowing.

pub mod body;
pub mod declared;
pub mod env;
pub mod expr;
pub mod flavors;
pub mod narrow;
pub mod query;
pub mod reuse;
pub mod runtime;
pub mod subst;
pub mod types;

use crate::{Analysis, Export, FileInput, FileKind, Target};
use body::{Checker, FileInfo};
use query::Mode;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::{HashMap, HashSet};
use types::{Field, Record, Ty, TypeId, Types};
use wowlua_diagnostics::Diagnostic;
use wowlua_syntax::view::{Block, ExprKind, ImportClause, RetType, StmtKind, Type, TypeArg};

/// How one file is checked (spec §2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileSettings {
    pub mode: Mode,
    /// Pure Lua not opted into strict mode: name and type diagnostics are
    /// warnings.
    pub pure_lua: bool,
}

/// Type-checks the files that have settings (`None`: not checked), with
/// the declarations of the package. `extra_globals`: `check.globals` and
/// SavedVariables.
pub fn check_package(
    files: &[FileInput],
    analysis: &Analysis,
    settings: &[Option<FileSettings>],
    extra_globals: &HashSet<String>,
    shared: Option<&flavors::SharedFlavors>,
) -> Vec<Diagnostic> {
    check_package_full(files, analysis, settings, extra_globals, shared, None, &[]).diagnostics
}

/// What checking a package produces.
#[derive(Debug, Default)]
pub struct CheckOutput {
    pub diagnostics: Vec<Diagnostic>,
    /// The static types of `typeName(expr)` arguments (`wowlua:debug`,
    /// spec §7.1), by the argument's span.
    pub type_names: HashMap<wowlua_span::Span, String>,
    /// Runtime checks (spec §7.5) of annotated parameters (by the name's
    /// span), `...: T` (by the `...` span) and `::` (by the expression's).
    pub value_checks: HashMap<wowlua_span::Span, runtime::ValueCheck>,
    /// Name spans of the definitions of renamable methods (see
    /// `env::Member::renamable`) and of the accesses that resolve to them.
    pub private_methods: HashSet<wowlua_span::Span>,
}

/// Like [`check_package`], with the type information lowering uses. With
/// `reuse`, plain Lua files take their checks from earlier flavors of the
/// build where they check the same, and store theirs for later ones (#6).
pub fn check_package_full(
    files: &[FileInput],
    analysis: &Analysis,
    settings: &[Option<FileSettings>],
    extra_globals: &HashSet<String>,
    shared: Option<&flavors::SharedFlavors>,
    mut reuse: Option<reuse::Reuse<'_>>,
    global_defs: &[Option<Vec<String>>],
) -> CheckOutput {
    let mut type_names = HashMap::new();
    let mut value_checks = HashMap::new();
    use types::DefId;
    let mut private_methods: HashMap<wowlua_span::Span, DefId> = HashMap::new();
    let mut global_classes: HashSet<DefId> = HashSet::new();
    // Declaration sources: declaration files first, then modules.
    let mut sources: Vec<env::Source> = Vec::new();
    let index: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.src_rel.as_str(), i))
        .collect();
    for (i, f) in files.iter().enumerate() {
        if f.kind == FileKind::Declaration
            && let Some(parse) = f.parse
        {
            let declares = f
                .src_rel
                .strip_suffix(".d.wowlua")
                .filter(|_| f.src_rel.starts_with(crate::STDLIB_PREFIX))
                .and_then(|stem| index.get(format!("{stem}.wowlua").as_str()).copied());
            sources.push(env::Source {
                index: i,
                parse,
                declaration: true,
                imports: Vec::new(),
                namespaces: Vec::new(),
                enums: FxHashMap::default(),
                declares,
                globals: f.globals,
            });
        }
    }
    for (i, f) in files.iter().enumerate() {
        if f.kind != FileKind::Wowlua {
            continue;
        }
        let (Some(parse), Some(info)) = (f.parse, &analysis.modules[i]) else {
            continue;
        };
        let mut imports = Vec::new();
        let mut namespaces = Vec::new();
        for s in parse.view().block().stmts() {
            let StmtKind::Import(import) = s.kind() else {
                continue;
            };
            let Some(resolved) = info.imports.iter().find(|r| r.start == s.span().start) else {
                continue;
            };
            let Target::Module(m) = resolved.target else {
                continue;
            };
            match &import.clause() {
                ImportClause::Names(specs) => {
                    for spec in specs {
                        let local = spec.alias.as_ref().unwrap_or(&spec.name);
                        imports.push((local.text().into(), m, spec.name.text().into()));
                    }
                }
                ImportClause::Namespace(n) => namespaces.push((n.text().into(), m)),
                ImportClause::SideEffect => {}
            }
        }
        let enums = info
            .enums
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.members
                        .iter()
                        .map(|(n, c)| (n.clone(), c.clone()))
                        .collect(),
                )
            })
            .collect();
        sources.push(env::Source {
            index: i,
            parse,
            declaration: false,
            imports,
            namespaces,
            enums,
            declares: None,
            globals: Default::default(),
        });
    }
    let mut env = env::Env::build(&sources);
    let mut diagnostics = std::mem::take(&mut env.diagnostics);
    let registry = files
        .iter()
        .position(|f| f.src_rel == format!("{}callbacks.wowlua", crate::STDLIB_PREFIX))
        .and_then(|i| env.module_types.get(&i))
        .and_then(|t| t.get("Registry"))
        .copied();
    env.declared.set_callbacks_registry(registry);
    let router = files
        .iter()
        .position(|f| f.src_rel == format!("{}slash.wowlua", crate::STDLIB_PREFIX))
        .and_then(|i| env.module_types.get(&i))
        .and_then(|t| t.get("Router"))
        .copied();
    env.declared.set_slash_router(router);
    // Shared mode: APIs only some flavors declare are known (as `any`), so
    // that their uses report E2030.
    if let Some(shared) = shared {
        for path in shared.availability.keys() {
            if path.starts_with("event:") {
                continue;
            }
            match path.rsplit_once('.') {
                None => {
                    env.declared.set_global_if_new(path, Types::ANY);
                }
                Some((ns, member)) => {
                    if let Some(d) = env.declared.global_type(ns)
                        && env.def(d).kind == env::DefKind::Namespace
                        && !env.def(d).members.contains_key(member)
                    {
                        env.def_mut(d).members.insert(
                            member.into(),
                            env::Member {
                                ty: Types::ANY,
                                kind: env::MemberKind::Field,
                                optional: false,
                                readonly: true,
                                visibility: env::Visibility::Public,
                                renamable: false,
                            },
                        );
                    }
                }
            }
        }
    }
    // Globals created with `@global` are declared (spec §4.3); globals the
    // package assigns are known (their assignments are W1002).
    let mut extra_globals: FxHashSet<String> = extra_globals.iter().cloned().collect();
    let mut assigned_globals = FxHashSet::default();
    let markers = crate::usage::Markers::default();
    // They depend on nothing but the file's parse: later flavors of the
    // build take them from the first (#6).
    let mut cached = reuse.as_mut().map(|r| &mut r.cache.globals);
    for (i, f) in files.iter().enumerate() {
        let Some(parse) = f.parse else { continue };
        let wowlua = f.kind == FileKind::Wowlua;
        let checked = settings.get(i).copied().flatten().is_some();
        if !wowlua && !checked {
            continue;
        }
        let mut fresh = FileGlobals::default();
        let globals = match cached.as_mut() {
            Some(c) => c
                .entry((f.src_rel.as_str().into(), parse.view().span().file))
                .or_default(),
            None => &mut fresh,
        };
        globals.fill(
            parse,
            wowlua,
            checked,
            &markers,
            global_defs.get(i).and_then(Option::as_deref),
            f.prepared.as_deref(),
        );
        #[cfg(debug_assertions)]
        {
            let mut again = FileGlobals::default();
            again.fill(parse, wowlua, checked, &markers, None, None);
            assert_eq!(
                (
                    wowlua.then_some(&again.decorated),
                    checked.then_some(&again.assigned)
                ),
                (
                    wowlua.then_some(&globals.decorated),
                    checked.then_some(&globals.assigned)
                ),
                "{} has other globals than it had",
                f.src_rel
            );
        }
        if wowlua {
            extra_globals.extend(globals.decorated.iter().flatten().cloned());
        }
        if checked {
            assigned_globals.extend(globals.assigned.iter().flatten().cloned());
        }
    }
    let extra_globals = &extra_globals;
    let assigned_globals = &assigned_globals;

    // Modules before their importers, so export types are known. The
    // modules of an import cycle are checked together, in source order;
    // the ones checked first see the others' exports as their annotations
    // declare them.
    let mut exports: FxHashMap<usize, FxHashMap<Box<str>, TypeId>> = FxHashMap::default();
    let checked = |i: usize| {
        settings.get(i).copied().flatten().is_some()
            && files[i].parse.is_some()
            && matches!(files[i].kind, FileKind::Wowlua | FileKind::Lua)
    };
    let mut order = Vec::with_capacity(files.len());
    for component in crate::order::components_in_order(files.len(), &analysis.edges) {
        if component.len() > 1 {
            for &m in component
                .iter()
                .filter(|&&m| checked(m) && files[m].kind == FileKind::Wowlua)
            {
                if let Some(parse) = files[m].parse {
                    let early = annotated_exports(&mut env, parse, m);
                    exports.insert(m, early);
                }
            }
        }
        order.extend(component);
    }
    for i in order {
        let (Some(settings), Some(parse)) = (settings.get(i).copied().flatten(), files[i].parse)
        else {
            continue;
        };
        if !checked(i) {
            continue;
        }
        let ctx = FileCtx {
            files,
            analysis,
            extra_globals,
            assigned_globals,
            shared,
            exports: &exports,
        };
        // A plain Lua file reads nothing of other files but the
        // declarations: a later flavor whose declarations give the same
        // answers to what it read gets the same results (#6).
        let plain_lua = files[i].kind == FileKind::Lua;
        let reusable = plain_lua && shared.is_none();
        if reusable
            && let Some(reuse) = reuse.as_mut()
            && reuse.cache.package_matches(extra_globals, assigned_globals)
            && let Some(reused) = reuse.cache.lookup(
                &env,
                (&files[i].src_rel, parse.view().span().file),
                settings,
            )
        {
            // Debug builds (and so the tests) check it again and compare.
            #[cfg(debug_assertions)]
            {
                let cp = env.types.checkpoint();
                let mut again = check_file_in(&mut env, &ctx, i, settings, parse);
                env.types.rollback(cp);
                let sorted = |mut v: Vec<(wowlua_span::Span, String)>| {
                    v.sort_by_key(|(s, _)| (s.start, s.end));
                    v
                };
                again.type_names = sorted(again.type_names);
                assert_eq!(
                    format!(
                        "{:?}",
                        (
                            &again.diagnostics,
                            &again.type_names,
                            again.value_checks.len(),
                            again.private_methods.len(),
                            again.global_classes.len(),
                            again.exports.len(),
                        )
                    ),
                    format!(
                        "{:?}",
                        (
                            &reused.diagnostics,
                            sorted(reused.type_names.clone()),
                            0,
                            0,
                            0,
                            0
                        )
                    ),
                    "{} is reused but checks differently",
                    files[i].src_rel
                );
            }
            reuse.cache.hits += 1;
            type_names.extend(reused.type_names);
            diagnostics.extend(reused.diagnostics);
            exports.insert(i, FxHashMap::default());
            continue;
        }
        // A plain Lua file leaves no type behind (its outputs are text,
        // spans and declarations): the arena goes back to how it was
        // before, so every file is checked from the same state, whatever
        // was checked before it.
        let before = plain_lua.then(|| env.types.checkpoint());
        #[cfg(debug_assertions)]
        let sizes = (
            env.def_count(),
            env.params.len(),
            env.typeof_types.len(),
            env.renamable_defs.len(),
            value_checks.len(),
        );
        let recording = reusable && reuse.as_ref().is_some_and(|r| r.store);
        if recording {
            env.declared.record();
            env.types.watch_births();
        }
        let out = check_file_in(&mut env, &ctx, i, settings, parse);
        if recording && let Some(reuse) = reuse.as_mut() {
            reuse.cache.misses += 1;
            let reads = env.declared.recorded().unwrap_or_default();
            // Private methods and global classes (declarations of this
            // flavor) are not stored, nor a check with a union whose order
            // can depend on what came before in ways it cannot tell.
            if let Some(unborn) = env.types.births()
                && out.private_methods.is_empty()
                && out.global_classes.is_empty()
            {
                reuse.cache.store(
                    &env,
                    (&files[i].src_rel, parse.view().span().file),
                    settings,
                    &reads,
                    unborn,
                    &out.diagnostics,
                    &out.type_names,
                    (extra_globals, assigned_globals),
                );
            }
        }
        type_names.extend(out.type_names);
        value_checks.extend(out.value_checks);
        private_methods.extend(out.private_methods);
        global_classes.extend(out.global_classes);
        diagnostics.extend(out.diagnostics);
        if let Some(cp) = before {
            #[cfg(debug_assertions)]
            {
                let after = (
                    env.def_count(),
                    env.params.len(),
                    env.typeof_types.len(),
                    env.renamable_defs.len(),
                    value_checks.len(),
                );
                assert_eq!(after, sizes, "a plain Lua file left types behind");
                assert!(out.exports.is_empty(), "a plain Lua file exports");
            }
            env.types.rollback(cp);
        }
        exports.insert(i, out.exports);
    }
    private_methods.extend(env.renamable_defs.iter().copied());
    // Classes reachable from globals, and every class of their hierarchies,
    // keep their method names (`rename-private`).
    let mut reachable = global_classes;
    loop {
        let before = reachable.len();
        for i in 0..env.def_count() {
            let id = DefId(i as u32);
            let def = env.def_unrecorded(id);
            for &b in &def.bases {
                if let types::Ty::Named(base, _) = env.types.get(b).clone() {
                    if reachable.contains(&id) {
                        reachable.insert(base);
                    }
                    if reachable.contains(&base) {
                        reachable.insert(id);
                    }
                }
            }
        }
        if reachable.len() == before {
            break;
        }
    }
    let private_methods: HashSet<wowlua_span::Span> = private_methods
        .into_iter()
        .filter(|(_, owner)| !reachable.contains(owner))
        .map(|(span, _)| span)
        .collect();
    CheckOutput {
        diagnostics,
        type_names,
        value_checks,
        private_methods,
    }
}

/// The type names a type annotation refers to.
fn type_refs<'a>(t: Type<'a>, out: &mut Vec<&'a str>) {
    use wowlua_syntax::view::TypeKind;
    match t.kind() {
        TypeKind::Ref { path, args } => {
            if let Some(last) = path.last() {
                out.push(last.text());
            }
            for a in args {
                if let TypeArg::Type(t) = a {
                    type_refs(t, out);
                }
            }
        }
        TypeKind::Paren(t) | TypeKind::Optional(t) | TypeKind::Array(t) => type_refs(t, out),
        TypeKind::Union(ts) | TypeKind::Intersection(ts) => {
            ts.iter().for_each(|t| type_refs(t, out))
        }
        _ => {}
    }
}

/// What one file adds to the package's globals: the names its `@global`
/// declarations create (a `.wowlua` file), and the globals it assigns (a
/// checked file). `None` until computed.
#[derive(Default)]
pub(crate) struct FileGlobals {
    decorated: Option<Vec<String>>,
    assigned: Option<Vec<String>>,
}

impl FileGlobals {
    /// Computes the parts a file of this kind needs that are not known yet.
    fn fill(
        &mut self,
        parse: &wowlua_syntax::Parse,
        wowlua: bool,
        checked: bool,
        markers: &crate::usage::Markers,
        global_defs: Option<&[String]>,
        prepared: Option<&body::Prepared>,
    ) {
        if wowlua && self.decorated.is_none() {
            let mut names = Vec::new();
            global_decorated(parse.view().block(), &mut names);
            self.decorated = Some(names);
        }
        if checked && self.assigned.is_none() {
            // The load order's usage of the file has them (its markers
            // differ, which changes only deferred calls).
            let mut names = match global_defs {
                Some(defs) => defs.to_vec(),
                None => crate::usage::file_usage(parse, markers)
                    .defs
                    .into_iter()
                    .filter_map(|d| match d.key {
                        crate::usage::Key::Global(name) => Some(name),
                        _ => None,
                    })
                    .collect(),
            };
            // `CreateFrame(type, "Name", …)` creates the global `Name`.
            match prepared {
                Some(prepared) => names.extend(prepared.created_frames.iter().cloned()),
                None => names.extend(body::prepare(parse).created_frames),
            }
            self.assigned = Some(names);
        }
    }
}

/// Names of declarations decorated with `@global`.
fn global_decorated(block: Block<'_>, out: &mut Vec<String>) {
    use wowlua_syntax::view::ExprKind;
    for s in block.stmts() {
        let Some(d) = s.decorators().iter().find(|d| d.is_named("global")) else {
            continue;
        };
        let explicit = d
            .args()
            .and_then(|a| match a.exprs().first().map(|e| e.kind()) {
                Some(ExprKind::String(lit)) => {
                    Some(String::from_utf8_lossy(&lit.value()).into_owned())
                }
                _ => None,
            });
        if let Some(name) = explicit {
            out.push(name);
            continue;
        }
        let inner = match s.kind() {
            StmtKind::Export(i) => i.kind(),
            k => k,
        };
        match inner {
            StmtKind::Local { names, .. } => {
                out.extend(names.iter().map(|n| n.name.text().to_string()))
            }
            StmtKind::LocalFunction { name, .. } | StmtKind::Const { name, .. } => {
                out.push(name.text().to_string());
            }
            StmtKind::Function { name, .. } => {
                out.push(
                    name.method()
                        .unwrap_or_else(|| name.path().last().expect("a name"))
                        .text()
                        .to_string(),
                );
            }
            StmtKind::Class(c) => {
                out.push(c.name().text().to_string());
            }
            StmtKind::Enum(e) => {
                out.push(e.name().text().to_string());
            }
            _ => {}
        }
    }
}

/// What checking one file needs besides the environment.
struct FileCtx<'a> {
    files: &'a [FileInput<'a>],
    analysis: &'a Analysis,
    extra_globals: &'a FxHashSet<String>,
    assigned_globals: &'a FxHashSet<String>,
    shared: Option<&'a flavors::SharedFlavors>,
    exports: &'a FxHashMap<usize, FxHashMap<Box<str>, TypeId>>,
}

/// What checking one file gives.
struct FileOutcome {
    diagnostics: Vec<Diagnostic>,
    type_names: Vec<(wowlua_span::Span, String)>,
    value_checks: Vec<(wowlua_span::Span, runtime::ValueCheck)>,
    private_methods: Vec<(wowlua_span::Span, types::DefId)>,
    global_classes: Vec<types::DefId>,
    exports: FxHashMap<Box<str>, TypeId>,
}

/// Compile-time tests of `wowlua:test` (decision T219): a function with
/// `@expectError("E2001", …)` must produce exactly these errors (and these
/// warnings, of those it names); they are then dropped. Otherwise E8122
/// says what it produced.
fn expected_errors(
    intrinsics: &crate::stdlib::Intrinsics,
    parse: &wowlua_syntax::Parse,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for st in parse.view().block().stmts() {
        let (name, func_span) = match st.kind() {
            StmtKind::LocalFunction { name, func } => (name.text().to_string(), func.span()),
            StmtKind::Function { name, func } => (
                name.path()
                    .iter()
                    .map(|n| n.text())
                    .collect::<Vec<_>>()
                    .join("."),
                func.span(),
            ),
            _ => continue,
        };
        let Some(d) = st.decorators().iter().find(|d| {
            let path = d.path();
            let id = if let Some([n]) = path.exactly::<1>() {
                intrinsics.get(n.text(), None)
            } else if let Some([m, n]) = path.exactly::<2>() {
                intrinsics.get(m.text(), Some(n.text()))
            } else {
                None
            };
            id == Some("test.expectError")
        }) else {
            continue;
        };
        let mut wanted: Vec<String> = d
            .args()
            .map(|a| a.exprs())
            .unwrap_or_default()
            .iter()
            .filter_map(|e| match e.kind() {
                ExprKind::String(lit) => Some(String::from_utf8_lossy(&lit.value()).into_owned()),
                _ => None,
            })
            .collect();
        wanted.sort();
        let inside = |diag: &Diagnostic| {
            diag.labels.first().is_some_and(|l| {
                l.span.file == func_span.file
                    && l.span.start >= func_span.start
                    && l.span.end <= func_span.end
            })
        };
        // The errors inside, and the warnings it expects.
        let mut got: Vec<String> = diagnostics
            .iter()
            .filter(|diag| inside(diag))
            .filter_map(|diag| diag.code)
            .filter(|c| c.starts_with('E') || wanted.iter().any(|w| w == c))
            .map(str::to_string)
            .collect();
        got.sort();
        if got == wanted {
            diagnostics.retain(|diag| {
                !(inside(diag) && diag.code.is_some_and(|c| wanted.iter().any(|w| w == c)))
            });
        } else {
            let shown = |list: &[String]| {
                if list.is_empty() {
                    "none".to_string()
                } else {
                    list.join(", ")
                }
            };
            diagnostics.push(
                Diagnostic::error(
                    "E8122",
                    format!(
                        "`{name}` should not compile with {}; it gives {}",
                        shown(&wanted),
                        shown(&got)
                    ),
                )
                .with_label(d.span(), "expected here"),
            );
        }
    }
}

/// Checks file `i` in `env`.
fn check_file_in(
    env: &mut env::Env,
    ctx: &FileCtx<'_>,
    i: usize,
    settings: FileSettings,
    parse: &wowlua_syntax::Parse,
) -> FileOutcome {
    let mut out = FileOutcome {
        diagnostics: Vec::new(),
        type_names: Vec::new(),
        value_checks: Vec::new(),
        private_methods: Vec::new(),
        global_classes: Vec::new(),
        exports: FxHashMap::default(),
    };
    let imports = import_types(env, ctx.files, ctx.analysis, i, ctx.exports);
    let info = FileInfo {
        intrinsics: crate::stdlib::Intrinsics::of(ctx.analysis, i, parse),
        index: i,
        mode: settings.mode,
        pure_lua: settings.pure_lua,
        extra_globals: ctx.extra_globals,
        assigned_globals: ctx.assigned_globals,
        imports,
        is_module: ctx.analysis.modules[i]
            .as_ref()
            .is_some_and(|m| m.is_module),
        shared: ctx.shared,
        meta_flavor: ctx.analysis.modules[i]
            .as_ref()
            .map(|m| {
                m.meta_bindings
                    .iter()
                    .filter(|(_, k)| **k == crate::meta::MetaKind::Flavor)
                    .map(|(n, _)| n.clone())
                    .collect()
            })
            .unwrap_or_default(),
    };
    // What the driver prepared for the file, or prepared here.
    let fresh;
    let prepared = match ctx.files[i].prepared.as_deref() {
        Some(prepared) => prepared,
        None => {
            fresh = body::prepare(parse);
            &fresh
        }
    };
    let mut checker = Checker::new(env, &info, prepared);
    checker.check_file(parse);
    for arg in &prepared.type_name_args {
        if let Some(&t) = checker.expr_types.get(&(arg.start, arg.end)) {
            out.type_names.push((*arg, checker.env.display(t)));
        }
    }
    out.value_checks
        .extend(std::mem::take(&mut checker.value_checks));
    out.private_methods
        .extend(std::mem::take(&mut checker.private_methods));
    // `@global` declarations hand their classes to other addons: the
    // class itself, the types of its values, the types its signature
    // names.
    for st in parse.view().block().stmts() {
        if !st.decorators().iter().any(|d| d.is_named("global")) {
            continue;
        }
        let inner = match st.kind() {
            StmtKind::Export(i) => i,
            _ => st,
        };
        let mut names: Vec<&str> = Vec::new();
        match inner.kind() {
            StmtKind::Class(c) => names.push(c.name().text()),
            StmtKind::Local { names: ns, values } => {
                for v in values {
                    if let Some(&t) = checker.expr_types.get(&(v.span().start, v.span().end)) {
                        checker.note_global_classes(t);
                    }
                }
                for n in ns {
                    if let Some(t) = &n.ty {
                        type_refs(*t, &mut names);
                    }
                }
            }
            StmtKind::LocalFunction { func, .. } | StmtKind::Function { func, .. } => {
                for p in func.params() {
                    if let Some(t) = p.ty() {
                        type_refs(t, &mut names);
                    }
                }
                match func.ret() {
                    Some(RetType::Single(t)) => type_refs(t, &mut names),
                    Some(RetType::List { types, .. }) => {
                        types.iter().for_each(|t| type_refs(t, &mut names))
                    }
                    _ => {}
                }
            }
            StmtKind::Const { ty: Some(t), .. } => type_refs(t, &mut names),
            _ => {}
        }
        for name in names {
            let found = checker
                .env
                .module_types
                .get(&i)
                .and_then(|m| m.get(name))
                .or_else(|| checker.env.module_imports.get(&i).and_then(|m| m.get(name)))
                .copied()
                .or_else(|| checker.env.declared.global_type(name));
            if let Some(d) = found {
                out.global_classes.push(d);
            }
        }
    }
    out.global_classes
        .extend(std::mem::take(&mut checker.global_classes));
    out.exports = std::mem::take(&mut checker.exports);
    expected_errors(&info.intrinsics, parse, &mut checker.diagnostics);
    out.diagnostics.append(&mut checker.diagnostics);
    out.diagnostics.append(&mut env.diagnostics);
    // A standard-library module's declarations give its exports as
    // callers see them (the compiler fills in some arguments).
    // (A declaration typed `any` only binds an intrinsic.)
    if let Some(declared) = env.declared_exports.get(&i) {
        out.exports.extend(
            declared
                .iter()
                .filter(|(_, t)| **t != Types::ANY)
                .map(|(k, &t)| (k.clone(), t)),
        );
    }
    out
}

/// The exports of module `m` whose types its annotations give without
/// checking it: functions (as hoisted, spec §5.1) and annotated locals and
/// constants. Undecorated only: a decorator can replace the value.
fn annotated_exports(
    env: &mut env::Env,
    parse: &wowlua_syntax::Parse,
    m: usize,
) -> FxHashMap<Box<str>, TypeId> {
    let scope = env::Scope {
        module: Some(m),
        ..env::Scope::default()
    };
    // The module's own check reports what its annotations get wrong.
    let reported = env.diagnostics.len();
    let mut out = FxHashMap::default();
    for s in parse.view().block().stmts() {
        let StmtKind::Export(inner) = s.kind() else {
            continue;
        };
        if !s.decorators().is_empty() || !inner.decorators().is_empty() {
            continue;
        }
        match inner.kind() {
            StmtKind::Function { name, func }
                if name.path().len() == 1 && name.method().is_none() =>
            {
                let sig = env.func_sig(&env::FnSig::of(func), &scope, false);
                let t = env.types.func(sig);
                out.insert(name.path().at(0).text().into(), t);
            }
            StmtKind::Local { names, .. } => {
                for n in names.iter() {
                    if let Some(t) = n.ty {
                        let t = env.resolve(t, &scope);
                        out.insert(n.name.text().into(), t);
                    }
                }
            }
            StmtKind::Const {
                name, ty: Some(t), ..
            } => {
                let t = env.resolve(t, &scope);
                out.insert(name.text().into(), t);
            }
            _ => {}
        }
    }
    env.diagnostics.truncate(reported);
    out
}

/// The types of a file's imported bindings.
fn import_types(
    env: &mut env::Env,
    files: &[FileInput],
    analysis: &Analysis,
    file: usize,
    exports: &FxHashMap<usize, FxHashMap<Box<str>, TypeId>>,
) -> FxHashMap<Box<str>, TypeId> {
    let mut out = FxHashMap::default();
    let (Some(parse), Some(info)) = (files[file].parse, &analysis.modules[file]) else {
        return out;
    };
    for s in parse.view().block().stmts() {
        let StmtKind::Import(import) = s.kind() else {
            continue;
        };
        if import.type_only() {
            continue;
        }
        let Some(resolved) = info.imports.iter().find(|r| r.start == s.span().start) else {
            continue;
        };
        let module = match resolved.target {
            Target::Module(m) => Some(m),
            _ => None,
        };
        let internal = import.internal();
        let export_type = |env: &mut env::Env, name: &str| -> TypeId {
            let Some(m) = module else { return Types::ANY };
            if internal {
                let key = format!("{}{name}", body::INTERNAL);
                return exports
                    .get(&m)
                    .and_then(|e| e.get(key.as_str()))
                    .copied()
                    .unwrap_or(Types::ANY);
            }
            if let Some(&t) = exports.get(&m).and_then(|e| e.get(name)) {
                return t;
            }
            match analysis.modules[m]
                .as_ref()
                .and_then(|i| i.exports.get(name))
            {
                Some(Export::Class(_) | Export::Enum(_)) => {
                    match env.module_types.get(&m).and_then(|t| t.get(name)).copied() {
                        Some(d) => env.types.intern(Ty::ClassOf(d)),
                        None => Types::ANY,
                    }
                }
                Some(Export::Const(Some(c))) => match c {
                    crate::constant::Const::Number(n) => env.types.num(*n),
                    crate::constant::Const::String(s) => env.types.str(&String::from_utf8_lossy(s)),
                    crate::constant::Const::Bool(true) => Types::TRUE,
                    crate::constant::Const::Bool(false) => Types::FALSE,
                    _ => Types::ANY,
                },
                _ => Types::ANY,
            }
        };
        match &import.clause() {
            ImportClause::Names(specs) => {
                for spec in specs {
                    let local = spec.alias.as_ref().unwrap_or(&spec.name);
                    let t = export_type(env, spec.name.text());
                    out.insert(local.text().into(), t);
                }
            }
            ImportClause::Namespace(n) => {
                let t = match module {
                    Some(m) => {
                        let names: Vec<String> = analysis.modules[m]
                            .as_ref()
                            .map(|i| i.exports.keys().cloned().collect())
                            .unwrap_or_default();
                        let mut record = Record::default();
                        for name in names {
                            let ty = export_type(env, &name);
                            record.fields.insert(
                                name.into(),
                                Field {
                                    ty,
                                    optional: false,
                                    readonly: true,
                                },
                            );
                        }
                        env.types.record(record)
                    }
                    None => Types::ANY,
                };
                out.insert(n.text().into(), t);
            }
            ImportClause::SideEffect => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wowlua_span::SourceDb;

    /// Checks one `.wowlua` source with the compiler's declarations.
    pub(crate) fn check_source(src: &str, mode: Mode) -> Vec<Diagnostic> {
        check_with(src, mode, None, None)
    }

    /// Checks with an extra declaration file and shared flavor mode.
    fn check_with(
        src: &str,
        mode: Mode,
        extra: Option<&'static str>,
        shared: Option<&flavors::SharedFlavors>,
    ) -> Vec<Diagnostic> {
        let mut db = SourceDb::new();
        let mut parses = Vec::new();
        let mut decls = wowlua_stdlib_files();
        decls.extend(extra.map(|text| ("<test>/extra.d.wowlua", text)));
        for decl in decls {
            let file = db.add(decl.0, decl.1.to_string());
            let p = wowlua_syntax::parse(decl.1.as_bytes(), file);
            assert!(!p.has_errors(), "{}: {:?}", decl.0, p.diagnostics);
            parses.push((decl.0.to_string(), p));
        }
        let file = db.add("t.wowlua", src.to_string());
        let main = wowlua_syntax::parse(src.as_bytes(), file);
        assert!(!main.has_errors(), "{:?}", main.diagnostics);
        let mut inputs: Vec<FileInput> = parses
            .iter()
            .map(|(path, p)| FileInput {
                src_rel: path.clone(),
                kind: FileKind::Declaration,
                parse: Some(p),
                xml: None,
                meta: None,
                main_of: None,
                globals: Default::default(),
                usage: None,
                prepared: None,
            })
            .collect();
        inputs.push(FileInput {
            src_rel: "t.wowlua".into(),
            kind: FileKind::Wowlua,
            parse: Some(&main),
            xml: None,
            meta: None,
            main_of: None,
            globals: Default::default(),
            usage: None,
            prepared: None,
        });
        let analysis = crate::analyze(&inputs, None);
        let mut settings: Vec<Option<FileSettings>> = vec![None; inputs.len()];
        *settings.last_mut().expect("main") = Some(FileSettings {
            mode,
            pure_lua: false,
        });
        let mut d = analysis.diagnostics.clone();
        d.extend(check_package(
            &inputs,
            &analysis,
            &settings,
            &HashSet::new(),
            shared,
        ));
        d
    }

    /// Checks `src` as a plain Lua file in two flavors whose extra
    /// declarations are `a` and `b`, the first storing into the cache the
    /// second looks up in: whether the first stored the file and the second
    /// reused it, and the cache.
    fn reuse_between(src: &str, a: &str, b: &str) -> (bool, bool, reuse::CheckCache) {
        let mut db = SourceDb::new();
        let file = db.add("t.lua", src.to_string());
        let main = wowlua_syntax::parse(src.as_bytes(), file);
        assert!(!main.has_errors(), "{:?}", main.diagnostics);
        let mut cache = reuse::CheckCache::default();
        let mut stored = false;
        for (k, extra) in [a, b].into_iter().enumerate() {
            let mut decls = wowlua_stdlib_files();
            decls.push(("<test>/extra.d.wowlua", extra));
            let parses: Vec<(String, wowlua_syntax::Parse)> = decls
                .into_iter()
                .map(|(path, text)| {
                    let f = db.add(path, text.to_string());
                    let p = wowlua_syntax::parse(text.as_bytes(), f);
                    assert!(!p.has_errors(), "{path}: {:?}", p.diagnostics);
                    (path.to_string(), p)
                })
                .collect();
            let mut inputs: Vec<FileInput> = parses
                .iter()
                .map(|(path, p)| FileInput {
                    src_rel: path.clone(),
                    kind: FileKind::Declaration,
                    parse: Some(p),
                    xml: None,
                    meta: None,
                    main_of: None,
                    globals: Default::default(),
                    usage: None,
                    prepared: None,
                })
                .collect();
            inputs.push(FileInput {
                src_rel: "t.lua".into(),
                kind: FileKind::Lua,
                parse: Some(&main),
                xml: None,
                meta: None,
                main_of: None,
                globals: Default::default(),
                usage: None,
                prepared: None,
            });
            let analysis = crate::analyze(&inputs, None);
            let mut settings: Vec<Option<FileSettings>> = vec![None; inputs.len()];
            *settings.last_mut().expect("main") = Some(FileSettings {
                mode: Mode::Basic,
                pure_lua: true,
            });
            check_package_full(
                &inputs,
                &analysis,
                &settings,
                &HashSet::new(),
                None,
                Some(reuse::Reuse {
                    cache: &mut cache,
                    store: k == 0,
                }),
                &[],
            );
            if k == 0 {
                stored = cache.stored() == 1;
            }
        }
        let hit = cache.hits == 1;
        (stored, hit, cache)
    }

    /// A plain Lua file is reused where what it read is the same, and only
    /// there (debug builds check every reuse again and compare).
    #[test]
    fn plain_lua_is_reused_where_its_reads_are_the_same() {
        let reused = |src: &str, a: &str, b: &str| {
            let (stored, hit, _) = reuse_between(src, a, b);
            assert!(stored, "not stored: {src}\n{a}");
            hit
        };
        let src = "print(Foo() + 1)\n";
        let foo = "declare function Foo(): number\n";
        assert!(reused(src, foo, foo), "the same declarations");
        assert!(
            reused(
                src,
                "declare function Foo(): number\ndeclare function Bar(): number\n",
                "declare function Foo(): number\ndeclare function Bar(): string\n",
            ),
            "a global it does not read differs"
        );
        assert!(
            !reused(src, foo, "declare function Foo(): string\n"),
            "a global it reads changes type"
        );
        assert!(
            !reused(src, foo, "declare function Other(): number\n"),
            "a global it reads is only in one flavor"
        );
        let ns = |a: &str, b: &str| {
            format!("declare namespace C_X\n    function A(): {a}\n    function B(): {b}\nend\n")
        };
        let reads_a = "print(C_X.A() + 1)\n";
        assert!(
            reused(reads_a, &ns("number", "number"), &ns("number", "string")),
            "a member it does not read changes"
        );
        assert!(
            !reused(reads_a, &ns("number", "number"), &ns("string", "number")),
            "a member it reads changes"
        );
        assert!(
            !reused(
                reads_a,
                &ns("number", "number"),
                "declare namespace C_X\n    function B(): number\nend\n",
            ),
            "a member it reads is missing"
        );
        // The API's `Enum`: namespaces of constants.
        let api_enum = |quality: &str, other: &str| {
            format!(
                "declare namespace Enum\n    namespace Quality\n{quality}    end\n    namespace Other\n{other}    end\nend\n"
            )
        };
        let reads_poor = "print(Enum.Quality.Poor + 1)\n";
        let poor_good = "        const Poor: 0\n        const Good: 1\n";
        assert!(
            reused(
                reads_poor,
                &api_enum(poor_good, "        const A: 0\n"),
                &api_enum(
                    "        const Poor: 0\n        const Good: 1\n        const Epic: 2\n",
                    "        const B: 0\n",
                ),
            ),
            "an enum it reads gains a member, another enum differs"
        );
        assert!(
            !reused(
                reads_poor,
                &api_enum(poor_good, "        const A: 0\n"),
                &api_enum(
                    "        const Poor: 1\n        const Good: 2\n",
                    "        const A: 0\n"
                ),
            ),
            "the member it reads has another value"
        );
        assert!(
            !reused(
                "local v = V\nprint(v)\n",
                "declare global V: \"a\" | \"b\"\n",
                "declare global W: \"b\"\ndeclare global V: \"a\" | \"b\"\n",
            ),
            "an API union's members are in another order"
        );
    }

    /// The names a plain Lua file looks up are recorded, found or not:
    /// globals, a namespace whose member it reads, and the addon namespace
    /// of `...`.
    #[test]
    fn plain_lua_records_what_it_looks_up() {
        let src = "local _, ns = ...\nns.x = 1\nprint(Foo(), Missing, string.format(\"%d\", 1))\n";
        let foo = "declare function Foo(): number\n";
        let (stored, _, cache) = reuse_between(src, foo, foo);
        assert!(stored);
        let roots = cache.roots();
        for root in [
            declared::Root::Global("Foo".into()),
            declared::Root::Global("Missing".into()),
            declared::Root::Global("print".into()),
            declared::Root::Global("string".into()),
        ] {
            assert!(roots.contains(&root), "{root:?} not in {roots:#?}");
        }
        assert!(
            roots
                .iter()
                .any(|r| matches!(r, declared::Root::Type(n) if &**n == env::ADDON_NAMESPACE)),
            "the addon namespace: {roots:#?}"
        );
    }

    fn wowlua_stdlib_files() -> Vec<(&'static str, &'static str)> {
        vec![(
            "<wowlua>/lua.d.wowlua",
            include_str!("../../../wowlua-stdlib/decl/lua.d.wowlua"),
        )]
    }

    fn codes(d: &[Diagnostic]) -> Vec<String> {
        d.iter()
            .map(|x| format!("{}: {}", x.code.unwrap_or("-"), x.message))
            .collect()
    }

    #[track_caller]
    fn ok(src: &str) {
        let d = check_source(src, Mode::Basic);
        assert!(d.iter().all(|x| !x.is_error()), "{src}\n{:#?}", codes(&d));
    }

    #[track_caller]
    fn err(src: &str, code: &str) {
        let d = check_source(src, Mode::Basic);
        assert!(
            d.iter().any(|x| x.code == Some(code)),
            "expected {code}: {src}\n{:#?}",
            codes(&d)
        );
    }

    #[test]
    fn flavor_guards() {
        let decl = "declare namespace C_X\n    function A(): number\n    function B(): number\nend\ndeclare global WOW_PROJECT_ID: number\n";
        let shared = flavors::SharedFlavors {
            flavors: ["standard".to_string(), "vanilla".into(), "mists".into()].into(),
            project_ids: [
                (1, "standard".to_string()),
                (2, "vanilla".into()),
                (19, "mists".into()),
            ]
            .into(),
            project_constants: HashMap::new(),
            availability: [
                (
                    "C_X".to_string(),
                    ["standard".to_string(), "mists".into()].into(),
                ),
                (
                    "C_X.A".to_string(),
                    ["standard".to_string(), "mists".into()].into(),
                ),
                ("C_X.B".to_string(), ["standard".to_string()].into()),
            ]
            .into(),
        };
        let run = |src: &str| -> Vec<String> {
            check_with(src, Mode::Basic, Some(decl), Some(&shared))
                .iter()
                .filter(|d| d.code == Some("E2030"))
                .map(|d| d.message.clone())
                .collect()
        };
        assert_eq!(
            run(
                "if C_X then print(C_X.A()) end\nif C_X ~= nil then print(C_X.A()) end\nlocal a = C_X and C_X.A()\nif not C_X or C_X.A() > 1 then end\nif WOW_PROJECT_ID == 1 then print(C_X.B()) end\n"
            ),
            Vec::<String>::new()
        );
        // The namespace exists in `mists`, which lacks `B`.
        assert_eq!(run("if C_X then print(C_X.B()) end\n").len(), 1);
        let unguarded = run("print(C_X.A())\n");
        assert!(
            unguarded
                .iter()
                .all(|m| m.contains("`vanilla`") && !m.contains("mists")),
            "{unguarded:?}"
        );
        assert!(run("if WOW_PROJECT_ID ~= 2 then print(C_X.B()) end\n")[0].contains("`mists`"));
        // Single-flavor builds: feature detection of what the flavor lacks.
        let d = check_with(
            "if C_X.Missing then C_X.Missing(1) end\nlocal f = C_X.Gone and C_X.Gone()\nif not OldApi then return end\nOldApi(1)\n",
            Mode::Strict,
            Some(decl),
            None,
        );
        assert!(d.is_empty(), "{:#?}", codes(&d));
        let d = check_with(
            "C_X.Missing()\nprint(OldApi)\n",
            Mode::Strict,
            Some(decl),
            None,
        );
        assert_eq!(
            d.iter().filter(|d| d.is_error()).count(),
            2,
            "{:#?}",
            codes(&d)
        );
    }

    /// Checker gaps found by the typed sample addon.
    #[test]
    fn sample_addon_rules() {
        // A generic class used before its declaration.
        strict_ok(
            "class Holder\n    ring: Ring<number>\n    constructor() self.ring = Ring.new() end\nend\nclass Ring<T>\n    items: T[] = {}\nend\nlocal h = Holder.new()\nprint(h.ring.items[1] + 1)\n",
        );
        // Top-level module functions are hoisted locals, not globals.
        strict_ok(
            "export function format(n: number): string return tostring(n) end\nfunction helper(): number return 1 end\nprint(format(helper()))\n",
        );
        // A function passed as an argument is a callback (no E2050).
        strict_ok("local function run(f: function) end\nrun(function(a, b) print(a, b) end)\n");
        strict_err("local f = function(a) print(a) end\nf(1)\n", "E2050");
        // `local x: T` without a value: `nil` until assigned.
        strict_ok(
            "local function f(b: boolean): number\n    local x: number\n    if b then x = 1 else x = 2 end\n    return x\nend\nprint(f(true))\n",
        );
        strict_err(
            "local function f(b: boolean): number\n    local x: number\n    if b then x = 1 end\n    return x\nend\nprint(f(true))\n",
            "E2001",
        );
        // An annotated `const` table takes the fields its type declares
        // or its indexer allows, as with `local`.
        strict_ok(
            "local function f(): any\n    const t: { [string]: any } = { x = 1 }\n    t.y = 2\n    return t\nend\nprint(f())\n",
        );
        strict_ok(
            "type R = { a: number, b?: string }\nlocal function f(): R\n    const r: R = { a = 1 }\n    print(r)\n    r.b = \"x\"\n    return r\nend\nprint(f())\n",
        );
        // Forward declarations with a type are usable in closures.
        strict_ok(
            "local update: () -> ()\nlocal function tick() update() end\nupdate = function() end\ntick()\n",
        );
        // A literal for a union of table types picks the member it fits.
        strict_ok(
            "type A = { kind: \"a\", n: number }\ntype B = { kind: \"b\" }\nlocal v: A | B = { kind = \"a\", n = 1 }\nv = { kind = \"b\" }\nprint(v)\n",
        );
        // Literal arrays take the expected element type.
        strict_ok(
            "local a: string[] = { \"x\", \"y\" }\nlocal m: { [string]: number } = { [\"k\"] = 1 }\nprint(table.concat(a, \",\"), m)\n",
        );
        // Constraints of inferred type arguments (E2014).
        err(
            "local function f<T extends string | number>(x: T): T return x end\nf(true)\n",
            "E2014",
        );
        ok("local function f<T extends string | number>(x: T): T return x end\nf(1)\n");
        // Array reads are `T` in strict mode too.
        strict_ok(
            "local function first(xs: number[]): number return xs[1] end\nprint(first({ 1 }))\n",
        );
    }

    #[test]
    fn unreachable_code() {
        let count = |src: &str| {
            check_source(src, Mode::Basic)
                .iter()
                .filter(|d| d.code == Some("W1005"))
                .count()
        };
        assert_eq!(
            count("local function f()\n    error(\"x\")\n    print(1)\n    print(2)\nend\n"),
            1
        );
        assert_eq!(
            count("do return end\nprint(1)\nif x then print(2) end\n"),
            1
        );
        assert_eq!(
            count(
                "local function f(x: boolean)\n    if x then return 1 else return 2 end\n    print(1)\nend\n"
            ),
            1
        );
        assert_eq!(
            count(
                "for i = 1, 2 do\n    if i then break end\n    print(i)\nend\nwhile true do end\nprint(1)\n"
            ),
            0
        );
    }

    #[test]
    fn basics() {
        ok("local x: number = 1\nlocal s = tostring(x)\nprint(s:upper(), #s, math.floor(1.5))\n");
        err("local x: number = \"a\"\n", "E2001");
        err("local function f(a: number) end\nf(\"x\")\n", "E2001");
        err("local function f(a: number) end\nf()\n", "E2007");
        err("local function f(a: number) end\nf(1, 2)\n", "E2006");
        let d = check_source(
            "local function f(a) end\nf(1, 2)\nundefinedThing()\n",
            Mode::Basic,
        );
        let c = codes(&d);
        assert!(
            c.iter().any(|x| x.starts_with("W2055")) && c.iter().any(|x| x.starts_with("W1001")),
            "{c:#?}"
        );
        err("local n = 1\nn()\n", "E2004");
        err("local t: { x: number } = { x = \"a\" }\n", "E2001");
        ok("local t = {}\nt.x = 1\nprint(t.x + 1)\n");
        ok(
            "for k, v in pairs({ a = 1 }) do print(k:upper(), v + 1) end\nfor i, v in ipairs({ \"a\" }) do print(i + 1, v:upper()) end\n",
        );
    }

    #[track_caller]
    fn strict_err(src: &str, code: &str) {
        let d = check_source(src, Mode::Strict);
        assert!(
            d.iter().any(|x| x.code == Some(code)),
            "expected {code}: {src}\n{:#?}",
            codes(&d)
        );
    }

    #[track_caller]
    fn strict_ok(src: &str) {
        let d = check_source(src, Mode::Strict);
        assert!(d.iter().all(|x| !x.is_error()), "{src}\n{:#?}", codes(&d));
    }

    /// WoW's Lua: no `os` (the error names the replacements), and the
    /// client's additions to `string`, `table` and `math` (decision S37).
    #[test]
    fn wow_lua_environment() {
        let d = check_source("print(os.time())\n", Mode::Strict);
        assert!(
            d.iter().any(|x| x.code == Some("E1001")
                && x.message.contains("WoW has no `os` library")
                && x.message.contains("time()")),
            "{:#?}",
            codes(&d)
        );
        strict_err("print(math.randomseed(1))\n", "E2003");
        strict_ok(
            "local s = \"wowlua\"\nlocal a: boolean = s:startswith(\"wow\") and s:endswith(\"lua\") and s:contains(\"wl\")\nlocal b: string = s:ltrim():rtrim(\"a\")\nlocal n: number = math.clamp(math.round(2.5), 0, 10) + math.lerp(0, 1, 0.5) + math.wrap(370, 0, 360)\nlocal list: string[] = { \"a\" }\nlocal i: number? = table.indexof(list, \"a\")\nlocal keys: string[] = table.keys({ x = 1 })\nlocal c: boolean = table.contains(list, \"a\") and table.isempty({})\nlocal removed: number = table.removevalue(list, \"a\")\nlocal last: string? = table.removeunordered(list, 1)\nlocal frozen: { x: number } = table.freeze({ x = 1 })\nprint(removed, last, frozen.x)\nlocal t: number = date(\"*t\").year + time() + difftime(2, 1)\nprint(a, b, n, i, keys, c, t)\n",
        );
    }

    /// A literal zero divisor is W2060 in every mode: WoW raises "Division
    /// by zero" (decision S38). A run-time zero cannot be seen.
    #[test]
    fn division_by_literal_zero() {
        for src in [
            "local x = 1\nprint(x / 0)\n",
            "local x = 1\nprint(x % 0.0)\n",
            "const ZERO = 0\nlocal x = 1\nprint(x / ZERO)\n",
            "local x = 1\nx /= 0\nprint(x)\n",
        ] {
            for mode in [Mode::Strict, Mode::Basic] {
                let d = check_source(src, mode);
                assert!(
                    d.iter().any(|x| x.code == Some("W2060")),
                    "{src}\n{:#?}",
                    codes(&d)
                );
            }
        }
        let src = "local x, y = 1, 0\nprint(x / y, x / 2, x % 3, math.huge)\n";
        let d = check_source(src, Mode::Strict);
        assert!(
            !d.iter().any(|x| x.code == Some("W2060")),
            "{src}\n{:#?}",
            codes(&d)
        );
    }

    /// A literal binds as its base type at the top level only: inside a
    /// table type the declared element type binds as it is (tables are
    /// invariant).
    #[test]
    fn generic_inference_keeps_declared_element_types() {
        strict_ok(
            "local m: { [string]: { [string]: string } | false } = {}\nfor _, v in pairs(m) do\n    if v then print(v.x) end\nend\n",
        );
        strict_ok("local l: (false | number)[] = {}\nfor _, v in ipairs(l) do print(v) end\n");
        // Top-level literals still widen: `T` is `string`, not `\"x\"`.
        strict_ok(
            "local function id<T>(v: T): T return v end\nlocal s = id(\"x\")\ns = \"y\"\nprint(s)\n",
        );
    }

    #[test]
    fn strict_mode() {
        strict_err("local function f(a) return a end\n", "E2050");
        strict_ok("local t: number[] = {}\ntable.sort(t, function(a, b) return a < b end)\n");
        strict_err(
            "local function f(x: string?) print(x:upper()) end\n",
            "E2005",
        );
        strict_err("local x: number = nil\n", "E2001");
        strict_err("local p: { x: number } = { x = 1, y = 2 }\n", "E2003");
        strict_err("class A\n x: number\n constructor() end\nend\n", "E2052");
        // A method of the class may read the field; a native one cannot.
        strict_err(
            "class A\n x: number\n constructor()\n  self:m()\n  self.x = 1\n end\n function m() print(self.x) end\nend\n",
            "E2052",
        );
        strict_ok(
            "declare class N\n    function SetSize(w: number, h: number)\nend\nclass A extends N\n x: number\n constructor()\n  super()\n  self:SetSize(1, 2)\n  self.x = 1\n end\nend\n",
        );
        strict_ok("class A\n x: number\n constructor() self.x = 1 end\nend\n");
        strict_err("print(undefinedGlobal)\n", "E1001");
        let d = check_source("pcall(function() coroutine.yield(1) end)\n", Mode::Basic);
        assert!(
            d.iter().any(|x| x.code == Some("W2057")),
            "{:#?}",
            codes(&d)
        );
    }

    #[test]
    fn phase4() {
        ok(
            "local Account = {}\nAccount.__index = Account\nfunction Account.new(b: number) return setmetatable({ balance = b }, Account) end\nfunction Account:deposit(v: number) self.balance = self.balance + v end\nlocal a = Account.new(1)\na:deposit(2)\nprint(a.balance + 1)\n",
        );
        ok(
            "local defaults = { scale = 1, locked = false }\nlocal s: typeof(defaults) = { scale = 2, locked = true }\n",
        );
        err(
            "local defaults = { scale = 1 }\nlocal s: typeof(defaults) = { scale = \"x\" }\n",
            "E2001",
        );
        ok("local function f(x: string?)\n assert(x)\n print(x:upper())\nend\n");
        ok(
            "local function f(x: string?): string\n if not x then error(\"no\") end\n return x\nend\n",
        );
        err(
            "class A\n function m() end\nend\nlocal a = A.new()\na.m()\n",
            "E2008",
        );
    }

    #[test]
    fn predicates() {
        ok(
            "local v: string | Secret<string> = \"x\"\nif issecretvalue(v) then return end\nprint(v:upper())\n",
        );
    }

    #[test]
    fn narrowing() {
        ok("local function f(x: string?)\n if x then print(x:upper()) end\nend\n");
        ok(
            "local function f(x: string | number)\n if type(x) == \"string\" then print(x:upper()) else print(x + 1) end\nend\n",
        );
        err(
            "local function f(x: string | number)\n print(x:upper())\nend\n",
            "E2003",
        );
        ok(
            "local function f(kind: \"a\" | \"b\")\n if kind == \"a\" then local y: \"a\" = kind end\nend\n",
        );
        ok("local x = 1\nx = \"s\"\nprint(x:upper())\n");
        err("local x = \"s\"\nx = 1\nprint(x:upper())\n", "E2003");
        // A local keeps a declared union of literals (a call's result);
        // fresh literals still widen, and the local takes other strings.
        ok(
            "type D = \"a\" | \"b\"\nlocal function f(): D return \"a\" end\nlocal function g(d: D) end\nlocal d = f()\ng(d)\nlocal s = \"a\"\ns = \"c\"\nlocal t = f()\nt = \"b\"\n",
        );
        err(
            "local s = \"a\"\nlocal function g(d: \"a\") end\ng(s)\n",
            "E2001",
        );
        // An optional chain with a value means its base exists.
        strict_ok(
            "type T = { link: string }\nlocal function f(t: T?)\n if t?.link ~= nil then print(t.link:upper()) end\n if t?.link then print(t.link) end\n if t?.link == \"x\" then print(t.link) end\n if t?.link == nil then return end\n print(t.link)\nend\n",
        );
        strict_err(
            "type T = { link: string }\nlocal function f(t: T?)\n if t?.link ~= nil then return end\n print(t.link)\nend\n",
            "E2005",
        );
        // `x?.f` narrows the field `x.f` too.
        strict_ok(
            "type O = { sound?: number }\nlocal function play(id: number) end\nlocal function f(o: O?)\n if o?.sound then play(o.sound) end\nend\n",
        );
        // A declared `any` is not narrowed to its initializer.
        ok(
            "interface S\n active: boolean\n function cancel()\nend\nlocal function f(): S\n local s: any = { active = true }\n s.cancel = function() end\n return s\nend\n",
        );
    }

    #[test]
    fn functions_and_generics() {
        ok("local function id<T>(v: T): T return v end\nlocal s: string = id(\"x\")\n");
        err(
            "local function id<T>(v: T): T return v end\nlocal s: number = id(\"x\")\n",
            "E2001",
        );
        err("local function f(): number return \"x\" end\n", "E2001");
        err(
            "local function f(x: boolean): number\n if x then return 1 end\nend\n",
            "E2009",
        );
        ok(
            "local ok, v = pcall(function(a: number) return a * 2 end, 1)\nif ok then print(v + 1) end\n",
        );
        // A function literal among the arguments a pack passes on.
        ok(
            "local function h(a: string, fn: () -> (), c: any?) end\nprint(pcall(h, \"x\", function() end))\n",
        );
        ok(
            "local t: number[] = {}\ntable.insert(t, 1)\nlocal n = select(\"#\", 1, 2)\nprint(n + 1)\n",
        );
        // A pack parameter of a function type (`(A...) -> R...`) binds the
        // parameters of a function passed for it, literals with annotated
        // parameters included.
        let wrap = "local function wrap<A..., R...>(fn: (A...) -> R...): (A...) -> R...\n    return fn\nend\n";
        err(
            &format!("{wrap}local w = wrap(function(a: number, b: string) end)\nw(\"x\", 1)\n"),
            "E2001",
        );
        ok(&format!(
            "{wrap}local w = wrap(function(a: number, b: string) end)\nw(1, \"x\")\n"
        ));
        err(
            &format!(
                "{wrap}local function named(a: number): string return tostring(a) end\nlocal s: number = wrap(named)(1)\n"
            ),
            "E2001",
        );
    }

    #[test]
    fn classes_and_interfaces() {
        ok(
            "class A\n x: number = 1\n function get(): number return self.x end\nend\nlocal a = A.new()\nprint(a:get() + a.x)\n",
        );
        err(
            "class A\n x: number = 1\nend\nlocal a = A.new()\nprint(a.y)\n",
            "E2003",
        );
        err(
            "class A\n private x: number = 1\nend\nprint(A.new().x)\n",
            "E2011",
        );
        ok(
            "interface P\n name: string\nend\nlocal p: P = { name = \"x\" }\nclass C\n name: string = \"c\"\nend\nlocal q: P = C.new()\n",
        );
        err(
            "interface P\n name: string\nend\nlocal p: P = { name = 1 }\n",
            "E2001",
        );
        ok(
            "class A\n function speak(): string return \"a\" end\nend\nclass B extends A\n override function speak(): string return super:speak() .. \"!\" end\nend\nlocal a: A = B.new()\n",
        );
        err(
            "class A\nend\nclass B\nend\nlocal a: A = B.new()\n",
            "E2001",
        );
        ok("enum E { X, Y }\nlocal e: E = E.X\nlocal n: number = E.Y\n");
        err("enum E { X, Y }\nlocal e: E = 3\n", "E2001");
    }

    /// Checks several `.wowlua` modules (path, source) of one package: the
    /// errors, as `code: message`.
    fn check_modules(modules: &[(&str, &str)], mode: Mode) -> Vec<String> {
        let mut db = SourceDb::new();
        let mut parses = Vec::new();
        for (path, text) in wowlua_stdlib_files() {
            let file = db.add(path, text.to_string());
            parses.push((
                path.to_string(),
                wowlua_syntax::parse(text.as_bytes(), file),
            ));
        }
        let decls = parses.len();
        for (path, text) in modules {
            let file = db.add(*path, text.to_string());
            let p = wowlua_syntax::parse(text.as_bytes(), file);
            assert!(!p.has_errors(), "{path}: {:?}", p.diagnostics);
            parses.push((path.to_string(), p));
        }
        let inputs: Vec<FileInput> = parses
            .iter()
            .enumerate()
            .map(|(i, (path, p))| FileInput {
                src_rel: path.clone(),
                kind: if i < decls {
                    FileKind::Declaration
                } else {
                    FileKind::Wowlua
                },
                parse: Some(p),
                xml: None,
                meta: None,
                main_of: None,
                globals: Default::default(),
                usage: None,
                prepared: None,
            })
            .collect();
        let analysis = crate::analyze(&inputs, None);
        let settings: Vec<Option<FileSettings>> = (0..inputs.len())
            .map(|i| {
                (i >= decls).then_some(FileSettings {
                    mode,
                    pure_lua: false,
                })
            })
            .collect();
        let mut d = analysis.diagnostics.clone();
        d.extend(check_package(
            &inputs,
            &analysis,
            &settings,
            &HashSet::new(),
            None,
        ));
        d.iter()
            .filter(|x| x.is_error())
            .map(|x| format!("{}: {}", x.code.unwrap_or("-"), x.message))
            .collect()
    }

    /// An export's type is the annotation of an annotated `const`, and a
    /// table has the fields assigned to it after its declaration, sealed
    /// (spec §2.4.2, §5.5).
    #[test]
    fn export_types() {
        // The annotation's optional fields, not the literal's sealed type.
        let tok = "export type Tok = { version: number, vision?: { [string]: number } }\nexport const a: Tok = { version = 2 }\n";
        let errs = check_modules(
            &[
                ("t.wowlua", tok),
                ("main.wowlua", "import a from \"./t\"\na.vision = {}\n"),
            ],
            Mode::Strict,
        );
        assert!(errs.is_empty(), "{errs:#?}");
        // A plain annotated `const` reads as its annotation too.
        strict_ok(
            "const H: { [string]: (...any) -> () } = { A = function() end }\nfor _, h in pairs(H) do h(1) end\n",
        );
        strict_ok("const N: number = 5\nconst M: 5 = N\nprint(M)\n");
        // Fields added after the literal are typed for importers, and the
        // table is sealed.
        let chat = "export const chat = { joined = false }\nfunction chat.Hide(n: number): number return n end\nchat.count = 1\nexport local other = {}\nfunction other.Show(): string return \"\" end\n";
        let errs = check_modules(
            &[
                ("t.wowlua", chat),
                (
                    "main.wowlua",
                    "import chat, other from \"./t\"\nlocal s: string = chat.Hide(1)\nlocal c: string = chat.count\nchat.Nope()\nlocal n: number = other.Show()\nprint(s, c, n, chat.joined)\n",
                ),
            ],
            Mode::Strict,
        );
        assert_eq!(errs.len(), 4, "{errs:#?}");
        assert!(errs[0].starts_with("E2001: `number`"), "{errs:#?}");
        assert!(errs[1].starts_with("E2001: `number`"), "{errs:#?}");
        assert!(
            errs[2].starts_with("E2003") && errs[2].contains("`Nope`"),
            "{errs:#?}"
        );
        assert!(errs[3].starts_with("E2001: `string`"), "{errs:#?}");
    }

    /// A callback taking a wider type than the other arguments bind does
    /// not widen the type argument (parameters are contravariant).
    #[test]
    fn callback_parameters_do_not_widen_type_arguments() {
        strict_ok(
            "local rows: { name: string, ms: number }[] = {}\ntable.sort(rows, function(a: { ms: number }, b: { ms: number }): boolean return a.ms > b.ms end)\n",
        );
        // Bound only by the callback, then by a later argument.
        strict_ok(
            "local function call<T>(f: (v: T) -> (), v: T) f(v) end\ncall(function(v: number?) end, 1)\n",
        );
        strict_err(
            "local function call<T>(f: (v: T) -> (), v: T) f(v) end\ncall(function(v: number) end, \"x\")\n",
            "E2001",
        );
        // A comparator that cannot take the elements still fails.
        strict_err(
            "local rows: { name: string }[] = {}\ntable.sort(rows, function(a: { ms: number }, b: { ms: number }): boolean return a.ms > b.ms end)\n",
            "E2001",
        );
        // A table with an indexer and named fields binds the key from both,
        // not from the callback.
        strict_ok(
            "local function keys<K, V>(t: { [K]: V }, less: (a: K, b: K) -> boolean): K[] return {} end\nlocal ks = keys({ [1] = true, b = true }, function(a: number | string, b: number | string): boolean return tostring(a) < tostring(b) end)\nprint(ks)\n",
        );
    }

    /// A table's own method replaces the one `setmetatable`'s `__index`
    /// gives (spec §2.4.6), as Lua finds it first.
    #[test]
    fn own_methods_replace_index_fallbacks() {
        let base = "local Base = {}\nfunction Base:Layout(count: number) print(count) end\nfunction Base:Other(): number return 1 end\nlocal Derived = setmetatable({}, { __index = Base })\n";
        strict_ok(&format!(
            "{base}function Derived:Layout() print(\"own\") end\nfunction Derived:Refresh() self:Layout() end\nlocal n: number = Derived:Other()\nprint(n)\n"
        ));
        strict_err(
            &format!("{base}function Derived:Layout() print(\"own\") end\nDerived:Layout(1)\n"),
            "E2006",
        );
        // An assigned field too, and the fallback still checks its calls.
        strict_ok(&format!(
            "{base}Derived.Layout = function(self: any, s: string) print(s) end\nDerived:Layout(\"x\")\n"
        ));
        strict_err(&format!("{base}Derived:Layout()\n"), "E2007");
    }

    /// Reading or writing a field of a function, number or boolean is an
    /// error however the key is written.
    #[test]
    fn indexing_a_function_is_an_error() {
        let g = "local function g(): number return 1 end\n";
        for access in [
            "g[1] = true\n",
            "g.x = 1\n",
            "print(g.y)\n",
            "print(g[2])\n",
            "local n = 1\nn.x = 2\n",
        ] {
            strict_err(&format!("{g}{access}"), "E2003");
        }
        strict_err("local f: function = print\nf.q = 1\n", "E2003");
        strict_ok(
            "local t: { [number]: boolean } = {}\nt[1] = true\nprint(t[1], (\"x\"):upper())\n",
        );
    }

    /// Mismatches say what does not fit: the missing field (and the `?:`
    /// form), the missing return value, the argument a function passed on
    /// lacks, the union member an assertion cannot cover.
    #[test]
    fn diagnostics_name_what_does_not_fit() {
        let messages = |src: &str| codes(&check_source(src, Mode::Strict));
        let d = messages(
            "local function f(opts: { layer: string?, sublevel: number? }?) print(opts) end\nf({ layer = \"BACKGROUND\" })\n",
        );
        assert_eq!(d.len(), 1, "{d:#?}");
        assert!(
            d[0].ends_with(": the field `sublevel` is missing (`sublevel: number?` must be present; write `sublevel?: number` for a field that may be absent)"),
            "{d:#?}"
        );
        let d = messages(
            "type O = { kind: string, size: number }\nlocal function f(o: O) print(o) end\nlocal x = { kind = \"x\", size = \"big\" }\nf(x)\n",
        );
        assert!(
            d.iter()
                .any(|m| m.ends_with(": the field `size` is `string`, not `number`")),
            "{d:#?}"
        );
        let d = messages(
            "local function rect(): (number?, number, number, number)\n    return nil\nend\nprint(rect())\n",
        );
        assert_eq!(
            d,
            vec![
                "E2001: missing return value 2 of type `number` (the function returns 4 values; one left out is `nil`)"
            ]
        );
        let d = messages(
            "local function j(name: string, b: number, voice: boolean) print(name, b, voice) end\nprint(pcall(j, \"x\"))\n",
        );
        assert_eq!(d, vec!["E2007: missing argument `b` of type `number`"]);
        // The given arguments are checked with a missing one; extra ones
        // are allowed, as Lua drops them.
        let d = messages(
            "local function j(name: string, n: number) print(name, n) end\nprint(pcall(j, 1))\nprint(pcall(j, \"x\", 2, 3))\n",
        );
        assert_eq!(
            d,
            vec![
                "E2001: `1` is not assignable to `string`",
                "E2007: missing argument `n` of type `number`"
            ]
        );
        let d = messages(
            "local function cast(pct: number | { r: number })\n    local p = pct :: number?\n    print(p)\nend\ncast(1)\n",
        );
        assert_eq!(
            d,
            vec![
                "E2015: cannot assert `number | { r: number }` as `number?` (neither type fits the other: `{ r: number }` does not fit `number?`, and `nil` does not fit `number | { r: number }`)"
            ]
        );
    }

    /// A field named by a string key may be optional (spec §2.2.4).
    #[test]
    fn optional_string_keyed_fields() {
        strict_ok(
            "type D = { kind: string, [\"not\"]?: boolean }\nlocal a: D = { kind = \"x\" }\nlocal b: D = { kind = \"x\", [\"not\"] = true }\nlocal n: boolean? = a[\"not\"]\nprint(b, n)\n",
        );
        strict_err(
            "type D = { kind: string, [\"not\"]?: boolean }\nlocal a: D = { kind = \"x\" }\nlocal n: boolean = a[\"not\"]\n",
            "E2001",
        );
        strict_err(
            "type D = { kind: string, [\"not\"]: boolean }\nlocal a: D = { kind = \"x\" }\n",
            "E2001",
        );
    }

    /// A list literal fits a record with a numeric indexer whose named
    /// fields are optional (spec §2.2.4), element by element.
    #[test]
    fn list_literals_for_records_with_a_numeric_indexer() {
        strict_ok(
            "type By = { [number]: any, talent?: boolean, what?: string }\nlocal x: By = { \"MAGE\" }\nlocal w: By = { \"DRUID\", 2, talent = true }\nprint(x, w)\n",
        );
        strict_ok(
            "type Ns = { [number]: string, n?: number }\nlocal ns: Ns = { \"a\", \"b\" }\nlocal s: string? = ns[1]\nprint(s)\n",
        );
        strict_err(
            "type Ns = { [number]: string, n?: number }\nlocal ns: Ns = { \"a\", 2 }\n",
            "E2001",
        );
        // A required named field is still required.
        strict_err(
            "type Ns = { [number]: string, n: number }\nlocal ns: Ns = { \"a\" }\n",
            "E2001",
        );
    }

    /// `{ [any]: any }` is every table (spec §2.6): namespaces, class and
    /// enum tables iterate with `pairs` (spec §3.4.3).
    #[test]
    fn every_table_is_a_map_of_any() {
        strict_ok("enum E { X, Y }\nfor k, v in pairs(E) do print(k, v) end\n");
        strict_ok(
            "class C\n x: number = 1\nend\nfor k, v in pairs(C) do print(k, v) end\nfor k, v in pairs(C.new()) do print(k, v) end\n",
        );
        strict_ok("for k, v in pairs(string) do print(k, v) end\n");
        // An enum value is a number, not a table.
        strict_err(
            "enum E { X, Y }\nlocal m: { [any]: any } = E.X\nprint(m)\n",
            "E2001",
        );
        // `table` stays a map of `any` keys only (spec §2.6).
        strict_err(
            "local t: table = {}\nlocal m: { [string]: any } = t\nprint(m)\n",
            "E2001",
        );
    }
}
