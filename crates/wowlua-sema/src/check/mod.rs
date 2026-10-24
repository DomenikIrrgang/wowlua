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
    // Their exported functions' counts (W2062).
    let mut export_caps: FxHashMap<usize, FxHashMap<Box<str>, usize>> = FxHashMap::default();
    // Modules of a cycle not checked yet: `exports` holds only what their
    // annotations give (W1036 where another reads the rest).
    let mut early: FxHashSet<usize> = FxHashSet::default();
    let checked = |i: usize| {
        settings.get(i).copied().flatten().is_some()
            && files[i].parse.is_some()
            && matches!(files[i].kind, FileKind::Wowlua | FileKind::Lua)
    };
    let mut order = Vec::with_capacity(files.len());
    let edges = check_edges(&env, analysis);
    for component in crate::order::components_in_order(files.len(), &edges) {
        if component.len() > 1 {
            for &m in component
                .iter()
                .filter(|&&m| checked(m) && files[m].kind == FileKind::Wowlua)
            {
                if let Some(parse) = files[m].parse {
                    let annotated = annotated_exports(&mut env, parse, m);
                    exports.insert(m, annotated);
                    early.insert(m);
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
            export_caps: &export_caps,
            early: &early,
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
        export_caps.insert(i, out.export_caps);
        early.remove(&i);
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
    // W2063 is the project's own: not of wowlua's library or a dependency
    // (the types those files declare are resolved with the environment).
    let foreign: FxHashSet<wowlua_span::FileId> = files
        .iter()
        .filter(|f| crate::is_compiler_known(&f.src_rel) || crate::package_of(&f.src_rel).is_some())
        .filter_map(|f| f.parse.map(|p| p.view().span().file))
        .collect();
    if !foreign.is_empty() {
        diagnostics.retain(|d| {
            d.code != Some("W2063")
                || !d
                    .labels
                    .first()
                    .is_some_and(|l| foreign.contains(&l.span.file))
        });
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
        TypeKind::Paren(t) | TypeKind::Optional(t) | TypeKind::Array(t) | TypeKind::KeyOf(t) => {
            type_refs(t, out)
        }
        TypeKind::Index { object, key } => {
            type_refs(object, out);
            type_refs(key, out);
        }
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
    export_caps: &'a FxHashMap<usize, FxHashMap<Box<str>, usize>>,
    early: &'a FxHashSet<usize>,
}

/// What checking one file gives.
struct FileOutcome {
    diagnostics: Vec<Diagnostic>,
    type_names: Vec<(wowlua_span::Span, String)>,
    value_checks: Vec<(wowlua_span::Span, runtime::ValueCheck)>,
    private_methods: Vec<(wowlua_span::Span, types::DefId)>,
    global_classes: Vec<types::DefId>,
    exports: FxHashMap<Box<str>, TypeId>,
    export_caps: FxHashMap<Box<str>, usize>,
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
        export_caps: FxHashMap::default(),
    };
    let imports = import_types(env, ctx, i, &mut out.diagnostics);
    let import_caps = import_caps(ctx, i);
    let info = FileInfo {
        intrinsics: crate::stdlib::Intrinsics::of(ctx.analysis, i, parse),
        index: i,
        mode: settings.mode,
        pure_lua: settings.pure_lua,
        extra_globals: ctx.extra_globals,
        assigned_globals: ctx.assigned_globals,
        imports,
        import_caps,
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
    // Closures see the unannotated locals a closure assigns as the union of
    // the values assigned to them: a file that has such reads is checked
    // again with the unions the pass before found, until they hold
    // (spec §2.4.1).
    let env_diagnostics = env.diagnostics.len();
    let mut seeds: Option<FxHashMap<u32, TypeId>> = None;
    let mut pass = 0;
    let mut forced = false;
    let mut checker = loop {
        pass += 1;
        env.diagnostics.truncate(env_diagnostics);
        env.typeof_namespace_refs.clear();
        let mut checker = Checker::new(env, &info, prepared);
        if let Some(s) = &seeds {
            checker.closure_seeds = s.clone();
        }
        checker.check_file(parse);
        if forced || !checker.closure_hit.get() {
            break checker;
        }
        let mut finals = checker.closure_finals();
        if seeds.as_ref() == Some(&finals) {
            break checker;
        }
        if pass >= 4 {
            // Still changing: those are `any`, one more pass.
            let prev = seeds.unwrap_or_default();
            for (d, t) in finals.iter_mut() {
                if prev.get(d) != Some(t) {
                    *t = Types::ANY;
                }
            }
            forced = true;
        }
        seeds = Some(finals);
    };
    // W2063 is the project's own: not of wowlua's library or a dependency.
    let rel = &ctx.files[i].src_rel;
    if crate::is_compiler_known(rel) || crate::package_of(rel).is_some() {
        checker.diagnostics.retain(|d| d.code != Some("W2063"));
    }
    // `M.T` through a namespace import, of a `typeof` alias not reached
    // yet: W1036 at the first use of each.
    let mut seen: FxHashSet<(usize, Box<str>)> = FxHashSet::default();
    for (m, name, span) in std::mem::take(&mut checker.env.typeof_namespace_refs) {
        if seen.insert((m, name.clone())) {
            out.diagnostics
                .push(typeof_alias_warning(ctx.files, m, &name, span));
        }
    }
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
    out.export_caps = std::mem::take(&mut checker.export_caps);
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

/// The import edges the check order follows: the load-time and run-time
/// ones, and a type-only import of a module with a `typeof` alias, which
/// has its type once that module is checked (spec §2.2.7), where it adds
/// no cycle.
fn check_edges(env: &env::Env, analysis: &Analysis) -> Vec<(usize, usize)> {
    let mut edges = analysis.edges.clone();
    let typeof_modules: FxHashSet<usize> = env.typeof_aliases.values().copied().collect();
    if typeof_modules.is_empty() {
        return edges;
    }
    for (i, info) in analysis.modules.iter().enumerate() {
        let Some(info) = info else { continue };
        for import in info.imports.iter().filter(|im| im.type_only) {
            let Target::Module(m) = import.target else {
                continue;
            };
            if m != i && typeof_modules.contains(&m) && !reaches(&edges, i, m) {
                edges.push((m, i));
            }
        }
    }
    edges
}

/// Is `to` reachable from `from` over `edges`?
fn reaches(edges: &[(usize, usize)], from: usize, to: usize) -> bool {
    let mut seen: FxHashSet<usize> = FxHashSet::default();
    let mut stack = vec![from];
    while let Some(n) = stack.pop() {
        if n == to {
            return true;
        }
        if seen.insert(n) {
            stack.extend(edges.iter().filter(|e| e.0 == n).map(|e| e.1));
        }
    }
    false
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
        // An export, or a file-level local (for `import internal`).
        let (inner, prefix) = match s.kind() {
            StmtKind::Export(inner) => (inner, ""),
            _ => (s, body::INTERNAL),
        };
        if !s.decorators().is_empty() || !inner.decorators().is_empty() {
            continue;
        }
        let key = |name: &str| -> Box<str> { format!("{prefix}{name}").into() };
        match inner.kind() {
            StmtKind::Function { name, func }
                if name.path().len() == 1 && name.method().is_none() =>
            {
                let sig = env.func_sig(&env::FnSig::of(func), &scope, false);
                let t = env.types.func(sig);
                out.insert(key(name.path().at(0).text()), t);
            }
            StmtKind::LocalFunction { name, func } if !prefix.is_empty() => {
                let sig = env.func_sig(&env::FnSig::of(func), &scope, false);
                let t = env.types.func(sig);
                out.insert(key(name.text()), t);
            }
            StmtKind::Local { names, .. } => {
                for n in names.iter() {
                    if let Some(t) = n.ty {
                        let t = env.resolve(t, &scope);
                        out.insert(key(n.name.text()), t);
                    }
                }
            }
            StmtKind::Const {
                name, ty: Some(t), ..
            } => {
                let t = env.resolve(t, &scope);
                out.insert(key(name.text()), t);
            }
            _ => {}
        }
    }
    env.diagnostics.truncate(reported);
    out
}

/// The counts of a file's imported functions (W2062), by local name: those
/// of modules checked before it (a cycle's later ones have none yet).
fn import_caps(ctx: &FileCtx<'_>, file: usize) -> FxHashMap<Box<str>, usize> {
    let mut out = FxHashMap::default();
    let (Some(parse), Some(info)) = (ctx.files[file].parse, &ctx.analysis.modules[file]) else {
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
        let Target::Module(m) = resolved.target else {
            continue;
        };
        let Some(caps) = ctx.export_caps.get(&m).filter(|_| m != file) else {
            continue;
        };
        if let ImportClause::Names(specs) = &import.clause() {
            for spec in specs {
                let key = if import.internal() {
                    format!("{}{}", body::INTERNAL, spec.name.text())
                } else {
                    spec.name.text().to_string()
                };
                if let Some(&cap) = caps.get(key.as_str()) {
                    let local = spec.alias.as_ref().unwrap_or(&spec.name);
                    out.insert(local.text().into(), cap);
                }
            }
        }
    }
    out
}

/// The types of a file's imported bindings. In an import cycle, an export
/// of a module checked later that its annotations do not type reads as
/// `any`: W1036 where it is imported or read (spec §5.4.4).
fn import_types(
    env: &mut env::Env,
    ctx: &FileCtx<'_>,
    file: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> FxHashMap<Box<str>, TypeId> {
    let (files, analysis, exports) = (ctx.files, ctx.analysis, ctx.exports);
    let mut out = FxHashMap::default();
    let (Some(parse), Some(info)) = (files[file].parse, &analysis.modules[file]) else {
        return out;
    };
    // Namespace imports of later modules of the cycle: local name, module.
    let mut later_namespaces: Vec<(&str, usize)> = Vec::new();
    for s in parse.view().block().stmts() {
        let StmtKind::Import(import) = s.kind() else {
            continue;
        };
        let Some(resolved) = info.imports.iter().find(|r| r.start == s.span().start) else {
            continue;
        };
        let module = match resolved.target {
            Target::Module(m) => Some(m),
            _ => None,
        };
        let other = module.filter(|&m| m != file);
        let later = other.filter(|m| ctx.early.contains(m));
        if import.type_only() {
            if let (Some(m), ImportClause::Names(specs)) = (other, import.clause()) {
                for spec in specs.iter() {
                    note_typeof_alias(env, files, m, spec.name, diagnostics);
                }
            }
            continue;
        }
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
                    if let Some(m) = later.filter(|_| !internal)
                        && untyped_in_cycle(analysis, exports, m, spec.name.text())
                    {
                        diagnostics.push(untyped_export(
                            files,
                            m,
                            spec.name.text(),
                            spec.name.span(),
                        ));
                    }
                    if let Some(m) = later.filter(|_| internal)
                        && untyped_internal(files, exports, m, spec.name.text())
                    {
                        diagnostics.push(untyped_internal_warning(
                            files,
                            m,
                            spec.name.text(),
                            spec.name.span(),
                        ));
                    }
                    if let Some(m) = other.filter(|_| !internal) {
                        note_typeof_alias(env, files, m, spec.name, diagnostics);
                    }
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
                if let Some(m) = later.filter(|_| !internal) {
                    later_namespaces.push((n.text(), m));
                }
            }
            ImportClause::SideEffect => {}
        }
    }
    // `ns.name` reads through a namespace import, once per name.
    if !later_namespaces.is_empty() {
        let mut seen: FxHashSet<(usize, &str)> = FxHashSet::default();
        wowlua_syntax::visit::walk_block(parse.view().block(), &mut |e| {
            let (ExprKind::Field { obj, name } | ExprKind::OptField { obj, name }) = e.kind()
            else {
                return;
            };
            let ExprKind::Name(ns) = obj.kind() else {
                return;
            };
            let Some(&(_, m)) = later_namespaces.iter().find(|(n, _)| *n == ns.text()) else {
                return;
            };
            if untyped_in_cycle(analysis, exports, m, name.text()) && seen.insert((m, name.text()))
            {
                diagnostics.push(untyped_export(files, m, name.text(), name.span()));
            }
        });
    }
    out
}

/// An export of module `m` (a module of the importer's cycle checked
/// after it) that reads as `any` until `m` is checked: a value its
/// annotations do not type.
fn untyped_in_cycle(
    analysis: &Analysis,
    exports: &FxHashMap<usize, FxHashMap<Box<str>, TypeId>>,
    m: usize,
    name: &str,
) -> bool {
    if exports.get(&m).is_some_and(|e| e.contains_key(name)) {
        return false;
    }
    matches!(
        analysis.modules[m]
            .as_ref()
            .and_then(|i| i.exports.get(name)),
        Some(Export::Value | Export::Function | Export::Const(None))
    )
}

/// W1036 for an untyped export of a later module of the cycle.
fn untyped_export(
    files: &[FileInput],
    m: usize,
    name: &str,
    span: wowlua_span::Span,
) -> Diagnostic {
    let module = crate::module_id(&files[m].src_rel);
    Diagnostic::warning(
        "W1036",
        format!(
            "`{name}` is `any` here: `{module}` is checked after this module in their import cycle, and its export `{name}` has no type annotation"
        ),
    )
    .with_label(span, "read as `any`")
    .with_help(if decorated_export(files, m, name) {
        format!(
            "its decorators give its type, known once `{module}` is checked: annotate it where it is read (`local f: T = {name}`)"
        )
    } else {
        format!("annotate it where `{module}` exports it (`export local {name}: T = …`)")
    })
}

/// A file-level local of module `m` (a module of the importer's cycle
/// checked after it) that `import internal` reads as `any` until `m` is
/// checked: one its annotations do not type.
fn untyped_internal(
    files: &[FileInput],
    exports: &FxHashMap<usize, FxHashMap<Box<str>, TypeId>>,
    m: usize,
    name: &str,
) -> bool {
    let key = format!("{}{name}", body::INTERNAL);
    if exports
        .get(&m)
        .is_some_and(|e| e.contains_key(key.as_str()))
    {
        return false;
    }
    let Some(parse) = files[m].parse else {
        return false;
    };
    parse.view().block().stmts().iter().any(|s| match s.kind() {
        StmtKind::Local { names, .. } => names.iter().any(|n| n.name.text() == name),
        StmtKind::LocalFunction { name: n, .. } => n.text() == name,
        StmtKind::Function { name: n, .. } => {
            n.path().len() == 1 && n.method().is_none() && n.path().at(0).text() == name
        }
        StmtKind::Const { name: n, .. } => n.text() == name,
        _ => false,
    })
}

/// W1036 for a file-level local of a later module of the cycle that
/// `import internal` reads.
fn untyped_internal_warning(
    files: &[FileInput],
    m: usize,
    name: &str,
    span: wowlua_span::Span,
) -> Diagnostic {
    let module = crate::module_id(&files[m].src_rel);
    Diagnostic::warning(
        "W1036",
        format!(
            "`{name}` is `any` here: `{module}` is checked after this module in their import cycle, and its local `{name}` has no type annotation"
        ),
    )
    .with_label(span, "read as `any`")
    .with_help(format!(
        "annotate it where `{module}` declares it (`local {name}: T = …`)"
    ))
}

/// Is the export `name` of module `m` decorated?
fn decorated_export(files: &[FileInput], m: usize, name: &str) -> bool {
    let Some(parse) = files[m].parse else {
        return false;
    };
    parse.view().block().stmts().iter().any(|s| {
        let StmtKind::Export(inner) = s.kind() else {
            return false;
        };
        (!s.decorators().is_empty() || !inner.decorators().is_empty())
            && crate::exported_names_of(inner).iter().any(|n| n == name)
    })
}

/// W1036 for an alias of a module checked later (in a cycle of imports,
/// type-only ones included) whose body uses `typeof(expr)`: it is `any`
/// until that module is checked.
fn note_typeof_alias(
    env: &env::Env,
    files: &[FileInput],
    m: usize,
    name: wowlua_syntax::view::Name<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(&def) = env.module_types.get(&m).and_then(|t| t.get(name.text())) else {
        return;
    };
    if env.typeof_aliases.get(&def) != Some(&m) {
        return;
    }
    diagnostics.push(typeof_alias_warning(files, m, name.text(), name.span()));
}

/// W1036 for the `typeof` alias `name` of module `m`, read at `span`.
fn typeof_alias_warning(
    files: &[FileInput],
    m: usize,
    name: &str,
    span: wowlua_span::Span,
) -> Diagnostic {
    let module = crate::module_id(&files[m].src_rel);
    Diagnostic::warning(
        "W1036",
        format!(
            "the type `{name}` is `any` here: `{module}` is checked after this module in their import cycle, and `{name}` is a `typeof(…)` it has not reached yet"
        ),
    )
    .with_label(span, "read as `any`")
    .with_help(format!("write the type out in `{module}` instead of `typeof(…)`"))
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
            "if C_X.Missing then C_X.Missing(1) end\nlocal _f = C_X.Gone and C_X.Gone()\nif not OldApi then return end\nOldApi(1)\n",
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
        // A defensive check whose other branch no value reaches does not
        // make the code after it unreachable.
        assert_eq!(
            count(
                "local function f(n: number): number\n    if not issecretvalue(n) then return n end\n    print(1)\n    return 0\nend\n"
            ),
            0
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

    /// `a and nil or b` (or `false` in the middle, in parentheses alike) is
    /// W2061 in every mode: it is always `b` (decision S39).
    #[test]
    fn and_falsy_or_is_always_the_last() {
        for src in [
            "local a, b = true, 1\nprint(a and nil or b)\n",
            "local a, b = true, 1\nprint((a and false) or b)\n",
            "local a, b = true, 1\nprint(a and (nil) or b)\n",
        ] {
            for mode in [Mode::Strict, Mode::Basic] {
                let d = check_source(src, mode);
                assert!(
                    d.iter().any(|x| x.code == Some("W2061")),
                    "{src}\n{:#?}",
                    codes(&d)
                );
            }
        }
        let src = "local a, b = true, 1\nprint(a and b or nil, not a and b or nil, a and 0 or b, a or nil)\n";
        let d = check_source(src, Mode::Strict);
        assert!(
            !d.iter().any(|x| x.code == Some("W2061")),
            "{src}\n{:#?}",
            codes(&d)
        );
    }

    /// A name past an `and` or `or` of a call is always nil: the operator
    /// keeps one value (W2062, decision L73).
    #[test]
    fn and_or_keep_one_value() {
        for src in [
            "local function xy(): (number, number) return 1, 2 end\nlocal p = true\nlocal x, y = p and xy()\nprint(x, y)\n",
            "local function xy(): (number, number) return 1, 2 end\nlocal x, y = nil, nil\nx, y = false or xy()\nprint(x, y)\n",
        ] {
            for mode in [Mode::Strict, Mode::Basic] {
                let d = check_source(src, mode);
                let w: Vec<_> = d.iter().filter(|x| x.code == Some("W2062")).collect();
                assert_eq!(w.len(), 1, "{src}\n{:#?}", codes(&d));
                assert!(
                    w[0].message.contains("`y` is always nil"),
                    "{}",
                    w[0].message
                );
            }
        }
        // one name, or a name starting with `_`, or no call: nothing
        for src in [
            "local function xy(): (number, number) return 1, 2 end\nlocal p = true\nlocal x = p and xy()\nprint(x)\n",
            "local function xy(): (number, number) return 1, 2 end\nlocal p = true\nlocal x, _ = p and xy()\nprint(x)\n",
            "local p = true\nlocal x, y = p and 1\nprint(x, y)\n",
            "local function xy(): (number, number) return 1, 2 end\nlocal p = true\nlocal x, y = (p and 1), xy()\nprint(x, y)\n",
        ] {
            let d = check_source(src, Mode::Strict);
            assert!(
                !d.iter().any(|x| x.code == Some("W2062")),
                "{src}\n{:#?}",
                codes(&d)
            );
        }
    }

    /// A value of several types at once fits a type of some of them: `A & B
    /// & C` is an `A & B` (and a `C & B`), not only an `A` (decision L74).
    #[test]
    fn intersections_fit_fewer_parts() {
        let classes = "class A\n    function a() end\nend\nclass B\n    function b() end\nend\nclass C\n    function c() end\nend\nlocal abc: A & B & C = A.new() :: unknown :: A & B & C\n";
        for (line, fits) in [
            ("local x: A & B = abc", true),
            ("local x: C & B = abc", true),
            ("local x: B = abc", true),
            ("local x: A & B & C = abc", true),
            ("local x: A & { n: number } = abc", false),
            ("local ab: A & B = abc\nlocal x: A & B & C = ab", false),
        ] {
            let src = format!("{classes}{line}\nprint(x)\n");
            for mode in [Mode::Strict, Mode::Basic] {
                let d = check_source(&src, mode);
                let errors: Vec<_> = d.iter().filter(|x| x.code == Some("E2001")).collect();
                assert_eq!(errors.is_empty(), fits, "{src}\n{:#?}", codes(&d));
            }
        }
    }

    /// A value read past what a `local function` can give (each `return` a
    /// fixed number of values) is W2062 in every mode: a name in a
    /// `local` or an assignment, or `select(n, …)` (decision S40).
    #[test]
    fn values_past_a_functions_results_are_nil() {
        // SuuUI's call helper: eight values at most, whatever it is declared
        let helper = "local function call(fn: any, ...: any): ...any
    if type(fn) ~= \"function\" then return nil end
    local ok, a, b, c, d, e, f, g, h = pcall(fn, ...)
    if ok then return a, b, c, d, e, f, g, h end
end
";
        for (body, word) in [
            (
                "local n, r, rs, lv, ml, ty, st, sc, el, tx, price = call(print)\nprint(n, r, rs, lv, ml, ty, st, sc, el, tx, price)\n",
                "`el`",
            ),
            ("print(select(11, call(print)))\n", "select(11)"),
        ] {
            let src = format!("{helper}{body}");
            for mode in [Mode::Strict, Mode::Basic] {
                let d = check_source(&src, mode);
                let w: Vec<_> = d.iter().filter(|x| x.code == Some("W2062")).collect();
                assert_eq!(w.len(), 1, "{src}\n{:#?}", codes(&d));
                assert!(
                    w[0].message.contains("at most 8 values") && w[0].message.contains(word),
                    "{}",
                    w[0].message
                );
            }
        }
        // a fixed function: an assignment, a value list ending in its call,
        // one that gives nothing
        for src in [
            "local function two() return 1, 2 end\nlocal x, y, z = 0, 0, 0\nx, y, z = two()\nprint(x, y, z)\n",
            "local function two() return 1, 2 end\nlocal a, b, c, d = 0, two()\nprint(a, b, c, d)\n",
            "local function none() print(1) end\nlocal v = none()\nprint(v)\n",
            // a `_` name past the count is skipped, the next one isn't
            "local function one() return 1 end\nlocal ok, _, link = one()\nprint(ok, link)\n",
        ] {
            let d = check_source(src, Mode::Strict);
            assert!(
                d.iter().any(|x| x.code == Some("W2062")),
                "{src}\n{:#?}",
                codes(&d)
            );
        }
        // no warning: up to the count; a return of open length (`...`, a
        // variadic call); the function replaced; another `select`; `'#'`
        for src in [
            &format!(
                "{helper}local a, b, c, d, e, f, g, h = call(print)\nprint(a, b, c, d, e, f, g, h, select(8, call(print)))\n"
            ) as &str,
            "local function okThen(ok: boolean, ...: any): ...any if ok then return ... end end\nlocal function call(fn: any, ...: any): ...any return okThen(pcall(fn, ...)) end\nlocal a, b, c, d, e, f, g, h, i, j, k, l = call(print)\nprint(a, b, c, d, e, f, g, h, i, j, k, l)\n",
            "local function two() return 1, 2 end\ntwo = function() return 1, 2, 3 end\nlocal a, b, c = two()\nprint(a, b, c)\n",
            "local function two() return 1, 2 end\nlocal select = function(_: number, ...: any): any return ... end\nprint(select(3, two()))\n",
            "local function two() return 1, 2 end\nprint(select('#', two()), select(2, two()))\n",
            // a name starting with `_` wants no value (a statement kept for
            // a decorator: `@if(…) local _x = f()`)
            "local function none() print(1) end\nlocal _kept = none()\n",
        ] {
            let d = check_source(src, Mode::Strict);
            assert!(
                !d.iter().any(|x| x.code == Some("W2062")),
                "{src}\n{:#?}",
                codes(&d)
            );
        }
    }

    /// W2062 beyond local functions (decision S41): a module's `function
    /// f()`, an imported function, the methods of a file-level table
    /// (`M:f()`, `M.f()`, `self:f()` in its methods); not a method set
    /// again, nor an API function.
    #[test]
    fn values_past_module_functions_and_methods_are_nil() {
        let w2062 = |modules: &[(&str, &str)]| -> Vec<String> {
            module_diagnostics(modules, Mode::Strict)
                .iter()
                .filter(|x| x.code == Some("W2062"))
                .map(|x| x.message.clone())
                .collect()
        };
        let lib = "export function pair(): (number, number) return 1, 2 end
export function open(...: any): ...any return ... end
";
        let main = "import pair, open from \"./lib\"
local M = {}
function M:two() return 1, 2 end
function M.one() return 1 end
function M:use()
    local a, b, c = self:two()
    local l1, l2 = self:late()
    print(a, b, c, l1, l2)
end
function M:late() return 1 end
local function f() return 1 end
local x, y = pair()
local p, q, r = pair()
local s, t = M:two()
local u, v = M.one()
local o1, o2, o3 = open(1, 2, 3)
print(x, y, p, q, r, s, t, u, v, o1, o2, o3, f)
";
        let got = w2062(&[("lib.wowlua", lib), ("main.wowlua", main)]);
        assert_eq!(got.len(), 4, "{got:#?}");
        assert!(
            got.iter()
                .any(|m| m.contains("`self:two` gives at most 2 values: `c`")),
            "{got:#?}"
        );
        // (defined further down: judged at the file's end)
        assert!(
            got.iter()
                .any(|m| m.contains("`self:late` gives at most 1 value: `l2`")),
            "{got:#?}"
        );
        assert!(
            got.iter()
                .any(|m| m.contains("`pair` gives at most 2 values: `r`")),
            "{got:#?}"
        );
        assert!(
            got.iter()
                .any(|m| m.contains("`M.one` gives at most 1 value: `v`")),
            "{got:#?}"
        );
        // a module's own `function f()`; a method set again loses its count;
        // an API function is never counted
        let own = "export function f() return 1 end
local a, b = f()
local M = {}
function M:two() return 1, 2 end
M.two = function() return 1, 2, 3 end
local c, d, e = M:two()
local name, realm, more = UnitName(\"player\")
print(a, b, c, d, e, name, realm, more)
";
        let got = w2062(&[("own.wowlua", own)]);
        assert_eq!(got.len(), 1, "{got:#?}");
        assert!(
            got[0].contains("`f` gives at most 1 value: `b`"),
            "{got:#?}"
        );
    }

    /// A call receiving a field path keeps that path's narrowing and its
    /// siblings'; it forgets the paths below it (decision L41).
    #[test]
    fn calls_on_a_field_keep_its_narrowing() {
        strict_ok(
            "declare class Tex\n    function SetSize(w: number, h: number)\nend\ndeclare function newTex(): Tex\nclass A\n    icon: Tex? = nil\n    other: Tex? = nil\n    constructor()\n        self.other = newTex()\n        self.icon = newTex()\n        self.icon:SetSize(1, 1)\n        self.icon:SetSize(2, 2)\n        self.other:SetSize(1, 1)\n    end\nend\nprint(A)\n",
        );
        // below it: forgotten
        strict_err(
            "local t: { a: { b: number? } } = { a = { b = nil } }\nlocal function f(x: { b: number? }) x.b = nil end\nt.a.b = 1\nf(t.a)\nlocal n: number = t.a.b\nprint(n)\n",
            "E2001",
        );
        // the root passed: all of it forgotten
        strict_err(
            "local t: { a: number? } = { a = nil }\nlocal function f(x: { a: number? }) x.a = nil end\nt.a = 1\nf(t)\nlocal n: number = t.a\nprint(n)\n",
            "E2001",
        );
    }

    /// Overloads take a last argument's several values (decision L40).
    #[test]
    fn overloads_expand_the_last_argument() {
        let pre =
            "declare function f(a: number, b: number, c: number)\ndeclare function f(a: string)\n";
        strict_ok(&format!(
            "{pre}local n: number[] = {{ 1, 2, 3 }}\nf(unpack(n))\nlocal function three(): (number, number, number) return 1, 2, 3 end\nf(three())\nf(1, unpack(n))\nf(\"x\")\n"
        ));
        strict_err(
            &format!("{pre}local b: boolean[] = {{}}\nf(unpack(b))\n"),
            "E2012",
        );
        strict_err(
            &format!("{pre}local function two(): (number, number) return 1, 2 end\nf(two())\n"),
            "E2012",
        );
    }

    /// A cast also compares the expression's type with its `nil` left out
    /// (decision L39).
    #[test]
    fn casts_compare_union_members() {
        let pre = "class A\n x: number = 1\nend\nclass B extends A\nend\nlocal bs: B[]? = nil\n";
        strict_ok(&format!("{pre}local as = bs :: A[]\nprint(as)\n"));
        strict_err(
            "local s: string? = nil\nlocal n = s :: number\nprint(n)\n",
            "E2015",
        );
        err(
            "local s: string? = nil\nlocal n = s :: number\nprint(n)\n",
            "E2015",
        );
        strict_err(
            "local v: string | boolean = true\nlocal n = v :: number\nprint(n)\n",
            "E2015",
        );
    }

    /// `assert` returns every argument, as Lua's does (decision L38): a
    /// message passed on is a second result, which a function declared to
    /// return one value reports; the first result is still the truthy value.
    #[test]
    fn assert_returns_its_arguments() {
        strict_err(
            "local function a(x: number?, k: string): number\n return assert(x, k)\nend\nprint(a)\n",
            "E2001",
        );
        strict_ok(
            "local function a(x: number?, k: string): number\n return (assert(x, k))\nend\nlocal function b(x: number?): number return assert(x) end\nlocal v, m = assert(a(1, \"k\"), \"why\")\nlocal s: string = m\nlocal n: number = v\nprint(b, s, n)\n",
        );
    }

    /// In a default every parameter is in scope, as in Lua: a name of the
    /// parameter itself or a later one is that parameter, not set yet
    /// (E1004), not an outer name it shadows; earlier parameters are set,
    /// and a function literal in a default runs later (decision L37).
    #[test]
    fn defaults_see_every_parameter() {
        strict_err(
            "local conf = { size = 1 }\nlocal function f(conf: { size: number } = conf): number return conf.size end\nprint(f)\n",
            "E1004",
        );
        strict_err(
            "local function f(x: number = y, y: number = 1): number return x + y end\nprint(f)\n",
            "E1004",
        );
        strict_err(
            "class Box\n constructor(private size: number = size) end\nend\nprint(Box)\n",
            "E1004",
        );
        strict_ok(
            "local function f(n: number, m: number = n + 1, g: (() -> number)? = function(): number return k end, k: number = 2): number return m + k + (g and g() or 0) end\nprint(f)\n",
        );
    }

    /// The expected type binds a type parameter only where it fits what
    /// the arguments bound (decision L36): a wider expected type widens it
    /// (strict mode's invariance needs that), a mismatch keeps the
    /// argument's type, so the error names the value's type alone.
    #[test]
    fn expected_type_binds_only_where_it_fits() {
        strict_ok(
            "local function list<T>(x: T): T[] return { x } end\nlocal xs: (number | string)[] = list(1)\nprint(xs)\n",
        );
        let d = check_source(
            "local function id<T>(x: T): T? return x end\nlocal n: boolean = id(5)\nprint(n)\n",
            Mode::Strict,
        );
        let got: Vec<&str> = d.iter().map(|x| x.message.as_str()).collect();
        assert_eq!(
            got,
            ["`number?` is not assignable to `boolean`"],
            "{got:#?}"
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
        // The message names the call the assignment comes after.
        let d = check_source(
            "class A\n x: number\n constructor()\n  self:m()\n  self.x = 1\n end\n function m() print(self.x) end\nend\n",
            Mode::Strict,
        );
        assert!(
            d.iter().any(|x| x.message
                == "field `x` is assigned only after `self:m()`, which may read it; assign it before"),
            "{:#?}",
            codes(&d)
        );
        strict_ok(
            "declare class N\n    function SetSize(w: number, h: number)\nend\nclass A extends N\n x: number\n constructor()\n  super()\n  self:SetSize(1, 2)\n  self.x = 1\n end\nend\n",
        );
        strict_ok("class A\n x: number\n constructor() self.x = 1 end\nend\n");
        // Every path: each branch of an `if` with an `else`, a `do` block.
        strict_ok(
            "class A\n x: number\n y: number\n constructor(b: boolean, c: boolean)\n  if b then self.x = 1 elseif c then self.x = 2 else self.x = 3 end\n  do self.y = 1 end\n end\nend\n",
        );
        strict_err(
            "class A\n x: number\n constructor(b: boolean)\n  if b then self.x = 1 end\n end\nend\n",
            "E2052",
        );
        strict_err(
            "class A\n x: number\n constructor(b: boolean, c: boolean)\n  if b then self.x = 1 elseif c then self.x = 2 else print(1) end\n end\nend\n",
            "E2052",
        );
        // a branch calling the class's method first: it may read the field
        strict_err(
            "class A\n x: number\n constructor(b: boolean)\n  if b then self:m() self.x = 1 else self.x = 2 end\n end\n function m() print(self.x) end\nend\n",
            "E2052",
        );
        // Parameter properties (decision L35): fields set from the
        // constructor's parameters, with their visibility and readonly.
        strict_ok(
            "interface Sink\n function say(t: string)\nend\nclass L\n constructor(private sink: Sink, public readonly name: string = \"n\", protected n: number?) end\n function f(): string self.sink:say(self.name) return self.name end\nend\nlocal l = L.new({ say = function(_: any, _: string) end })\nprint(l.name, l:f())\n",
        );
        strict_err(
            "class L\n constructor(private secret: string) end\nend\nprint(L.new(\"x\").secret)\n",
            "E2011",
        );
        strict_err(
            "class L\n constructor(readonly id: number) end\nend\nlocal l = L.new(1)\nl.id = 2\n",
            "E2010",
        );
        strict_err(
            "class L\n x: number = 1\n constructor(private x: number) end\nend\n",
            "E3046",
        );
        // A readonly field: assigned in its own class's constructor only.
        strict_err(
            "class A\n readonly id: number = 1\n function f() self.id = 2 end\nend\n",
            "E2010",
        );
        strict_err(
            "class A\n constructor(readonly id: number) end\nend\nclass B extends A\n constructor() super(1) self.id = 2 end\nend\n",
            "E2010",
        );
        strict_ok("class A\n readonly id: number = 1\n constructor() self.id = 2 end\nend\n");
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
        // `canaccessvalue` is the complement (spec §2.11): its true branch
        // drops the secret members, in an `if` and in an `and` chain, and
        // its false branch keeps only them.
        strict_ok(
            "local T: { [string]: string } = {}\nlocal function f(v: string | Secret<string>): string?\n if canaccessvalue(v) then return T[v] end\n return nil\nend\nlocal function g(c: (string | Secret<string>)?)\n local b = c and canaccessvalue(c) and c:lower()\n print(f, b)\nend\nprint(g)\n",
        );
        strict_err(
            "local function f(v: string | Secret<string>)\n if not canaccessvalue(v) then print(v:upper()) end\nend\nprint(f)\n",
            "E2040",
        );
        strict_ok(
            "local function f(v: string | Secret<string>)\n if not canaccessvalue(v) then return end\n print(v:upper())\nend\nprint(f)\n",
        );
        // A local named `canaccessvalue` is not the global.
        strict_err(
            "local function canaccessvalue(v: unknown): boolean return true end\nlocal function f(v: string | Secret<string>)\n if canaccessvalue(v) then print(v:upper()) end\nend\nprint(f)\n",
            "E2040",
        );
        // A generic predicate infers its type parameters from the argument
        // (`V | Secret<V> | nil` against `number | Secret<number>`), in
        // both branches.
        strict_ok(
            "local function plain<V>(v: V | Secret<V> | nil): v is V\n return (v :: any) ~= nil\nend\nlocal function use(n: number) end\nlocal function f(map: number?, level: number | Secret<number>)\n local a: number = plain(level) and level or 1\n if not plain(map) then return end\n use(map)\n use(a)\nend\nprint(f)\n",
        );
    }

    /// `ClassOf<T>` for a type parameter binds `T` to the class, generic
    /// class or enum whose table is passed (decision L36); inside the
    /// generic body it is a table. A generic predicate binds its type
    /// parameters from every argument, so `is(v, Dog)` narrows `v` to `Dog`.
    #[test]
    fn class_of_type_parameters() {
        let pre = "class Dog\n name: string = \"x\"\nend\nclass Box<T>\n item: T? = nil\nend\nenum Dir { Up, Down }\nlocal function from<T>(_c: ClassOf<T>, _v: any): T? return nil end\nlocal function is<T>(v: any, c: ClassOf<T>): v is T return v ~= c end\n";
        strict_ok(&format!(
            "{pre}local d: Dog? = from(Dog, 1)\nlocal b: Box<number>? = from(Box, 1)\nlocal e: Dir? = from(Dir, 2)\nlocal a = from({{}}, 1)\nlocal function keep<T>(c: ClassOf<T>): table return c end\nprint(d, b, e, a, keep(Dog))\n"
        ));
        strict_err(
            &format!("{pre}local n: number? = from(Dog, 1)\nprint(n)\n"),
            "E2001",
        );
        strict_err(
            &format!("{pre}local s: string? = from(Dir, 1)\nprint(s)\n"),
            "E2001",
        );
        strict_ok(&format!(
            "{pre}local function f(v: Dog | string): string\n if is(v, Dog) then return v.name end\n return v\nend\nprint(f)\n"
        ));
        strict_err(
            &format!(
                "{pre}local function f(v: Dog | string)\n if is(v, Dog) then local n: string = v print(n) end\nend\nprint(f)\n"
            ),
            "E2001",
        );
    }

    /// `cond and x` with a boolean `cond` is `false | x`: an inferred local
    /// keeps the `false` beside other types, so a truthiness test leaves
    /// `x` (spec §2.4), and so do the results `pcall` passes on.
    #[test]
    fn boolean_literals_beside_other_types() {
        strict_ok(
            "local function f(): boolean return math.random() > 0.5 end\nlocal t = { x = 1 }\nlocal y = f() and t\nif y then print(y.x) end\n",
        );
        strict_ok(
            "local function g(key: string): number\n local dir = (key == \"UP\" and 1) or (key == \"DOWN\" and -1)\n if not dir then return 0 end\n return dir * 2\nend\nprint(g)\n",
        );
        strict_ok(
            "local fixed = 1\nlocal c = fixed and fixed ~= 2 and tostring(fixed)\nif c then print(c:upper()) end\n",
        );
        strict_ok(
            "local function q(n: number): string | false\n if n > 1 then return \"x\" end\n return false\nend\nlocal ok, r = pcall(q, 2)\nif ok and r then print(r .. \"!\") end\n",
        );
        // An inferred result keeps it too.
        strict_ok(
            "local function f(ok: boolean, t: { x: number }) return ok and t end\nlocal y = f(true, { x = 1 })\nif y then print(y.x) end\n",
        );
        // A boolean alone still widens: the local may hold `true` later.
        strict_ok("local done = false\ndone = true\nlocal b: boolean = done\nprint(b)\n");
        // Literals a call's results carry stay literals past the first
        // name too.
        strict_ok(
            "local function k(): \"a\" | \"b\" return \"a\" end\nlocal function use(x: \"a\" | \"b\") end\nlocal ok, v = pcall(k)\nif ok then use(v) end\n",
        );
    }

    /// A declared local narrowed before a loop keeps its narrowing inside
    /// when no path back to the loop start widens it (spec §2.5); `break`
    /// paths join the flow after the loop.
    #[test]
    fn loop_narrowing() {
        strict_ok(
            "local function h(s: string?): number\n local n: number? = nil\n if s then\n  n = 0\n  for _ = 1, 3 do n = n + 1 end\n end\n return n or 1\nend\nprint(h)\n",
        );
        strict_ok(
            "local function h2(xs: number[]): number[]?\n local out: number[]? = {}\n for _, x in ipairs(xs) do\n  if x < 0 then out = nil break end\n  out[#out + 1] = x\n end\n return out\nend\nprint(h2)\n",
        );
        // After the loop, a `break` that set it to `nil` counts.
        strict_err(
            "local function h3(xs: number[]): number\n local out: number[]? = {}\n for _, x in ipairs(xs) do\n  if x < 0 then out = nil break end\n end\n return #out\nend\nprint(h3)\n",
            "E2005",
        );
        // A path back to the start that sets it to `nil`: the body sees
        // the declared type, through the end of the body or `continue`.
        strict_err(
            "local function w(): number\n local n: number? = 0\n while math.random() > 0.5 do\n  n = n + 1\n  n = nil\n end\n return 0\nend\nprint(w)\n",
            "E2001",
        );
        strict_err(
            "local function w(): number\n local n: number? = 0\n for i = 1, 3 do\n  if i == 2 then n = nil continue end\n  n = n + 1\n end\n return 0\nend\nprint(w)\n",
            "E2001",
        );
        strict_err(
            "local function w(): number\n local n: number? = 0\n repeat\n  n = n + 1\n  if n > 3 then n = nil end\n until math.random() > 0.5\n return 0\nend\nprint(w)\n",
            "E2001",
        );
        // The retried loop reports each problem once.
        let d = check_source(
            "local function w(): number\n local n: number? = 0\n while math.random() > 0.5 do\n  local s: string = 1\n  n = nil\n end\n return 0\nend\nprint(w)\n",
            Mode::Strict,
        );
        assert_eq!(
            d.iter().filter(|x| x.code == Some("E2001")).count(),
            1,
            "{:#?}",
            codes(&d)
        );
    }

    /// A field assignment through an annotated local checks against the
    /// declared members its narrowed value fits: `if item then item.x = 1`
    /// on an `Item | false` does not index `false`.
    #[test]
    fn field_assignment_through_narrowed_local() {
        strict_ok(
            "type Item = { [string]: any }\nlocal function f(k: number): Item[]\n local items: Item[] = {}\n for _ = 1, k do\n  local item: Item | false | nil\n  if k > 1 then item = { key = \"a\" } elseif k < 0 then item = false end\n  if item == false then break end\n  if item then\n   item.x = 1\n   items[#items + 1] = item\n  end\n end\n return items\nend\nprint(f)\n",
        );
        strict_err(
            "type Item = { x: number }\nlocal function f(item: Item | false)\n item.x = 1\nend\nprint(f)\n",
            "E2003",
        );
    }

    /// Closures see a local assigned after its declaration as declared
    /// (spec §2.5); E2005 on one says so.
    #[test]
    fn closure_nil_help() {
        let d = check_source(
            "type Opts = { onClick: () -> () }\nlocal function button(opts: Opts?)\n opts = opts or { onClick = function() end }\n local function run() opts.onClick() end\n run()\nend\nprint(button)\n",
            Mode::Strict,
        );
        assert!(
            d.iter().any(|x| x.code == Some("E2005")
                && x.help
                    .as_deref()
                    .is_some_and(|h| h.contains("`opts` is assigned after its declaration"))),
            "{d:#?}"
        );
    }

    /// Closures see an unannotated local a closure assigns as the union of
    /// the values assigned to it, settled over passes; a bare `local x`
    /// adds no `nil` of its own (spec §2.4.1, decision L47).
    #[test]
    fn closure_assigned_union() {
        // A flag saved and restored inside the closure: `boolean`, not `any`.
        strict_err(
            "local painting = false\nlocal function f()\n local was = painting\n painting = true\n local s: string = was\n painting = was\n print(s)\nend\nprint(f)\n",
            "E2001",
        );
        // A counter: `number`.
        strict_err(
            "local count = 0\nlocal function f(): string\n count = count + 1\n return count\nend\nprint(f)\n",
            "E2001",
        );
        strict_ok(
            "local count = 0\nlocal function f(): number\n count = count + 1\n return count\nend\nprint(f)\n",
        );
        // A forward declaration: the function assigned later, no `nil`.
        strict_ok(
            "local g\nlocal function f(): number return g() end\ng = function(): number return 1 end\nprint(f)\n",
        );
        strict_err(
            "local g\nlocal function f(): number return g() end\ng = function(): string return \"a\" end\nprint(f)\n",
            "E2001",
        );
        // A call where it is declared may have set it: still no `nil`.
        strict_ok(
            "type Layer = { n: number }\nlocal layer = nil\nlocal function build() layer = { n = 1 } :: Layer end\nbuild()\nlocal function show(): number\n if not layer then build() end\n return layer.n\nend\nprint(show)\n",
        );
        // What the closure itself assigned holds there.
        strict_ok(
            "local n = 0\nlocal function f(): string\n n = \"a\"\n return n\nend\nprint(f)\n",
        );
        // A `nil` a closure assigns counts.
        strict_err(
            "local n\nlocal function set() n = 1 end\nlocal function clear() n = nil end\nlocal function read(): number return n end\nprint(set, clear, read)\n",
            "E2001",
        );
        // Each problem once, from the last pass.
        let d = check_source(
            "local count = 0\nlocal function f()\n count = count + 1\n local s: string = count\n print(s)\nend\nprint(f)\n",
            Mode::Strict,
        );
        assert_eq!(
            d.iter().filter(|x| x.code == Some("E2001")).count(),
            1,
            "{:#?}",
            codes(&d)
        );
    }

    /// A callback in a table literal argument takes the type parameters
    /// the table's other fields bind; an expected `any` or `unknown` binds
    /// nothing (decision L48).
    #[test]
    fn generic_table_callbacks() {
        let seg = "type Opts<V> = { options: V[], onChange?: (v: V) -> () }\nlocal function seg<V extends string | number>(opts: Opts<V>): V? return opts.options[1] end\n";
        strict_ok(&format!(
            "{seg}local r = seg({{ options = {{ 1, 2 }}, onChange = function(v) print(v + 1) end }})\nprint(r)\n"
        ));
        strict_err(
            &format!(
                "{seg}local r = seg({{ options = {{ \"a\" }}, onChange = function(v) local n: number = v; print(n) end }})\nprint(r)\n"
            ),
            "E2001",
        );
        // In `print(…)` (`any`) and for an `unknown` parameter too.
        strict_err(
            &format!(
                "{seg}print(seg({{ options = {{ 1 }}, onChange = function(v) local s: string = v; print(s) end }}))\n"
            ),
            "E2001",
        );
        strict_ok(&format!(
            "{seg}local function take(x: unknown) print(x) end\ntake(seg({{ options = {{ 1 }} }}))\n"
        ));
    }

    /// Strict mode: an array's named field (not `n`) and a record's missing
    /// number key are E2003; other keys of a table without an indexer read
    /// `any` (W2063). Decision L49.
    #[test]
    fn unindexed_reads() {
        strict_err("local xs: number[] = { 1 }\nprint(xs.label)\n", "E2003");
        strict_ok("local xs: number[] = { 1 }\nprint(xs.n, xs[1])\n");
        strict_err(
            "type Art = { atlas: string }\nlocal a: Art = { atlas = \"x\" }\nprint(a[1])\n",
            "E2003",
        );
        strict_ok(
            "type Entry = { [number]: string, main?: string }\nlocal e: Entry = { \"a\", main = \"b\" }\nprint(e[1], e.main)\n",
        );
        // a dynamic key: `any`, W2063
        let d = check_source(
            "type Art = { atlas: string }\nlocal a: Art = { atlas = \"x\" }\nlocal function at(k: string) return a[k] end\nprint(at)\n",
            Mode::Strict,
        );
        assert!(d.iter().any(|x| x.code == Some("W2063")), "{d:#?}");
        // basic mode: as before
        let d = check_source("local xs: number[] = { 1 }\nprint(xs.label)\n", Mode::Basic);
        assert!(!d.iter().any(|x| x.code == Some("E2003")), "{d:#?}");
    }

    /// Strict mode: a string's member the `string` library doesn't have, and
    /// a literal's named entry (not `n`) where an array is expected, are
    /// E2003. Decision L71.
    #[test]
    fn strings_and_array_literals() {
        strict_err("local s: string = \"x\"\nprint(s.message)\n", "E2003");
        strict_err("local s: string = \"x\"\nprint(s:nope())\n", "E2003");
        strict_ok(
            "local s: string = \"x\"\nprint(s:upper(), s.len, (\"%d\"):format(1), s:trim())\n",
        );
        strict_err(
            "local roles: string[] = { class = \"DAMAGER\", \"TANK\" }\nprint(roles)\n",
            "E2003",
        );
        strict_err(
            "type R = { short: string[] }\nlocal r: R = { short = { class = \"BM\", \"MM\" } }\nprint(r)\n",
            "E2003",
        );
        strict_ok("local xs: number[] = { 1, 2, n = 2 }\nprint(xs)\n");
        strict_ok(
            "type L = { [number]: string, label?: string }\nlocal l: L = { \"a\", label = \"b\" }\nprint(l)\n",
        );
        // basic mode: as before
        let d = check_source(
            "local s: string = \"x\"\nprint(s.message)\nlocal xs: string[] = { a = \"b\" }\nprint(xs)\n",
            Mode::Basic,
        );
        assert!(!d.iter().any(|x| x.code == Some("E2003")), "{d:#?}");
    }

    /// `date("*t")` gives the time's parts, any other format a string
    /// (decision L72).
    #[test]
    fn date_is_typed() {
        strict_ok(
            "local t = date(\"*t\", 0)\nlocal y: number = t.year + t.yday\nlocal s: string = date(\"%H:%M\")\nprint(y, s, date(\"!*t\").hour)\n",
        );
        strict_err("local n: number = date(\"%H\")\nprint(n)\n", "E2001");
        strict_err("print(date(\"*t\").nope)\n", "E2003");
    }

    /// `x or {}` without an expected type takes `x`'s type where `{}` fits it
    /// (decision L52).
    #[test]
    fn empty_table_defaults() {
        strict_err(
            "type O = { n?: number }\nlocal function f(o: O?)\n local d = o or {}\n local s: string = d.n\n print(s)\nend\nprint(f)\n",
            "E2001",
        );
        strict_err(
            "local function g(l: number[]?)\n local x = l or {}\n local s: string = x[1]\n print(s)\nend\nprint(g)\n",
            "E2001",
        );
        strict_err(
            "type O = { n?: number }\nlocal function f(o: O?)\n local d = o ?? {}\n local s: string = d.n\n print(s)\nend\nprint(f)\n",
            "E2001",
        );
        // `{}` doesn't fit: the union, as before
        strict_ok("local function h(s: string?)\n local v = s or {}\n print(v)\nend\nprint(h)\n");
        strict_ok(
            "type R = { n: number }\nlocal function k(r: R?)\n local v = r or {}\n print(v)\nend\nprint(k)\n",
        );
    }

    /// A class declared after a `typeof` alias reads its type in its members
    /// (decision L57).
    #[test]
    fn typeof_alias_in_classes() {
        strict_err(
            "local defaults = { scale = 1 }\ntype Settings = typeof(defaults)\nclass Box\n s: Settings\n constructor(s: Settings) self.s = s end\n function Scale(): string return self.s.scale end\nend\nprint(Box)\n",
            "E2001",
        );
        strict_ok(
            "local defaults = { scale = 1 }\ntype Settings = typeof(defaults)\nclass Box\n s: Settings\n constructor(s: Settings) self.s = s end\n function Scale(): number return self.s.scale end\nend\nprint(Box)\n",
        );
    }

    /// A field annotated with `typeof(expr)` of its own has the
    /// expression's type where the file reaches the class, not `any`
    /// (decision L70).
    #[test]
    fn typeof_in_class_fields() {
        strict_err(
            "local defaults = { scale = 1 }\nclass Box\n s: typeof(defaults)? = nil\n function Scale(): string local s = self.s return s and s.scale or \"\" end\nend\nprint(Box)\n",
            "E2001",
        );
        let d = check_source(
            "local defaults = { scale = 1 }\nclass Box\n s: typeof(defaults)? = nil\n function Scale(): number local s = self.s return s and s.scale or 0 end\nend\nlocal b = Box.new()\nlocal kept = b.s\nprint(kept)\n",
            Mode::Strict,
        );
        assert!(
            d.iter().all(|x| !x.is_error() && x.code != Some("W2063")),
            "{d:#?}"
        );
    }

    /// Reads on an intersection: a field from the first part that has it,
    /// a key from the first part with numbered values (decision L60).
    #[test]
    fn intersection_reads() {
        let pre =
            "type Color = number[] & { hex: string }\nlocal c = { 1, 2, 3 } :: unknown :: Color\n";
        strict_err(
            &format!("{pre}local h: number = c.hex\nprint(h)\n"),
            "E2001",
        );
        strict_err(&format!("{pre}local r: string = c[1]\nprint(r)\n"), "E2001");
        let d = check_source(
            &format!(
                "{pre}local h: string = c.hex\nlocal r: number = c[2]\nlocal i = 3\nlocal b = c[i]\nprint(h, r, b)\n"
            ),
            Mode::Strict,
        );
        assert!(
            d.iter().all(|x| !x.is_error() && x.code != Some("W2063")),
            "{d:#?}"
        );
    }

    /// A record alias that refers to itself is a named structural type: its
    /// references have its fields, not `any` (decision L58).
    #[test]
    fn recursive_record_aliases() {
        let node = "type Node = { name?: string, kids?: Node[] }\n";
        strict_err(
            &format!(
                "{node}local function f(n: Node) for _, c in ipairs(n.kids or {{}}) do local x: number = c.name end end\nprint(f)\n"
            ),
            "E2001",
        );
        let d = check_source(
            &format!(
                "{node}local function f(n: Node) for _, c in ipairs(n.kids or {{}}) do local x: string? = c.name; print(x) end end\nprint(f)\n"
            ),
            Mode::Strict,
        );
        assert!(
            d.iter().all(|x| !x.is_error() && x.code != Some("W2063")),
            "{d:#?}"
        );
        // table literals at any depth
        strict_ok(&format!(
            "{node}local t: Node = {{ name = \"a\", kids = {{ {{ name = \"b\", kids = {{}} }} }} }}\nprint(t)\n"
        ));
        strict_err(
            &format!("{node}local t: Node = {{ kids = {{ {{ name = 5 }} }} }}\nprint(t)\n"),
            "E2001",
        );
        // with type parameters
        let tree = "type Tree<T> = { value: T, kids: Tree<T>[] }\n";
        strict_ok(&format!(
            "{tree}local t: Tree<number> = {{ value = 1, kids = {{ {{ value = 2, kids = {{}} }} }} }}\nfor _, k in ipairs(t.kids) do local v: number = k.value; print(v) end\n"
        ));
        strict_err(
            &format!(
                "{tree}local t: Tree<number> = {{ value = 1, kids = {{}} }}\nfor _, k in ipairs(t.kids) do local v: string = k.value; print(v) end\n"
            ),
            "E2001",
        );
        // values match as the record does: a map of unknowns takes it
        strict_ok(&format!(
            "{node}local function names(n: Node): {{ [string]: unknown }} return n end\nprint(names({{ name = \"a\" }}))\n"
        ));
        // aliases that refer to each other
        strict_err(
            "type A = { b?: B, n: number }\ntype B = { a?: A }\nlocal x: A = { n = 1 }\nlocal s: string = x.b and x.b.a and x.b.a.n or \"\"\nprint(s)\n",
            "E2001",
        );
    }

    /// A table assigned to an unannotated local: its empty `{}` fields take
    /// the local's field types where it fits them (decision L59).
    #[test]
    fn assigned_empty_fields() {
        let pre = "type B = { seen: { [number]: boolean }, ids: number[] }\nlocal by: { [string]: B } = {}\n";
        let d = check_source(
            &format!(
                "{pre}local function f(key: string)\n local p = by[key]\n if not p then\n  p = {{ seen = {{}}, ids = {{}}, extra = 1 }}\n  by[key] = p\n end\n local x: boolean? = p.seen[1]\n print(x, p.ids[1])\nend\nprint(f)\n"
            ),
            Mode::Strict,
        );
        assert!(
            d.iter().all(|x| !x.is_error() && x.code != Some("W2063")),
            "{d:#?}"
        );
        strict_err(
            &format!(
                "{pre}local function f(key: string)\n local p = by[key]\n if not p then p = {{ seen = {{}}, ids = {{}} }} end\n local x: string = p.seen[1]\n print(x)\nend\nprint(f)\n"
            ),
            "E2001",
        );
        // the value's own fields stay
        strict_ok(&format!(
            "{pre}local p = by.a\np = {{ seen = {{}}, ids = {{}}, extra = 1 }}\nlocal n: number = p.extra\nprint(n)\n"
        ));
        // a value that doesn't fit is taken as it is
        strict_ok("local t = { a = 1 }\nt = { b = {} }\nprint(t.b)\n");
    }

    /// A local keeps the type of a value written with `::` (decision L64).
    #[test]
    fn local_keeps_assertion() {
        strict_ok(
            "type P = \"TOP\" | \"BOTTOM\"\nlocal function f(s: string): P\n local p = (s ~= \"\" and s or \"TOP\") :: P\n return p\nend\nprint(f)\n",
        );
        strict_ok(
            "type P = \"TOP\" | \"BOTTOM\"\nlocal function f(s: string): P\n local p = (s :: P)\n return p\nend\nprint(f)\n",
        );
        // a literal still widens
        strict_err(
            "type P = \"TOP\" | \"BOTTOM\"\nlocal function f(): P\n local p = \"TOP\"\n p = \"x\" .. p\n return p\nend\nprint(f)\n",
            "E2001",
        );
    }

    /// A local without an annotation that a loop assigns keeps its types
    /// through the loop: its type before it and the ones the body gives
    /// (decision L65).
    #[test]
    fn loop_assigned_unannotated() {
        // after the loop: the type before it or the body's
        strict_err(
            "local function f(q: number): number\n local h = q > 1 and q or nil\n for i = 1, 3 do h = i end\n return h\nend\nprint(f)\n",
            "E2001",
        );
        strict_ok(
            "local function f(q: number): number?\n local h = q > 1 and q or nil\n for i = 1, 3 do h = i end\n local n: number? = h\n return n\nend\nprint(f)\n",
        );
        strict_err(
            "local function f(t: string[]): string\n local h = 0\n for _, s in ipairs(t) do h = s end\n return h\nend\nprint(f)\n",
            "E2001",
        );
        // in the body: what the paths back bring too
        strict_err(
            "local function f()\n local last = nil\n for _, v in ipairs({ 1, 2 }) do\n  local n: number = last\n  last = v\n  print(n)\n end\nend\nprint(f)\n",
            "E2001",
        );
        strict_ok(
            "local function f()\n local last = nil\n for _, v in ipairs({ 1, 2 }) do\n  local n: number? = last\n  last = v\n  print(n)\n end\nend\nprint(f)\n",
        );
        // a `break` path joins the flow after the loop
        strict_err(
            "local function f(t: string[]): number\n local found = nil\n for i, s in ipairs(t) do\n  if s == \"x\" then found = i break end\n end\n return found\nend\nprint(f)\n",
            "E2001",
        );
        // a counter stays a number
        strict_ok(
            "local function f(): number\n local n = 0\n while n < 10 do n = n + 1 end\n repeat n = n - 1 until n < 5\n return n\nend\nprint(f)\n",
        );
    }

    /// `unknown` where it is falsy is `false | nil` (decision L63).
    #[test]
    fn unknown_falsy() {
        strict_ok(
            "local function f(x: unknown)\n local a: boolean = x and true or false\n local c: number | false | nil = x and 1\n print(a, c)\nend\nprint(f)\n",
        );
        // a test narrows it
        strict_ok(
            "local function f(x: unknown)\n if not x then\n  local n: false | nil = x\n  print(n)\n end\nend\nprint(f)\n",
        );
        // where truthy it is still unknown
        strict_err(
            "local function f(x: unknown)\n local a: number = x or 1\n print(a)\nend\nprint(f)\n",
            "E2001",
        );
    }

    /// `typeof(expr)` in a function's parameter and result annotations
    /// reads the expression's type (decision L62).
    #[test]
    fn typeof_in_signatures() {
        let pre = "local C = { a = { hex = \"x\" }, b = { hex = \"y\" } }\n";
        strict_err(
            &format!(
                "{pre}local function f(t: typeof(C))\n local n: number = t\n print(n)\nend\nprint(f)\n"
            ),
            "E2001",
        );
        strict_err(
            &format!(
                "{pre}local function f(): typeof(C) return C end\nlocal n: number = f()\nprint(n)\n"
            ),
            "E2001",
        );
        // a key of its field names reads those fields
        let d = check_source(
            &format!(
                "{pre}local function hex(k: keyof typeof(C)): string return C[k].hex end\nprint(hex(\"a\"))\n"
            ),
            Mode::Strict,
        );
        assert!(
            d.iter().all(|x| !x.is_error() && x.code != Some("W2063")),
            "{d:#?}"
        );
        strict_err(
            &format!(
                "{pre}local function hex(k: keyof typeof(C)): string return C[k].hex end\nprint(hex(\"c\"))\n"
            ),
            "E2001",
        );
        // a function literal's
        strict_err(
            &format!(
                "{pre}local f = function(t: typeof(C)) local n: number = t print(n) end\nprint(f)\n"
            ),
            "E2001",
        );
    }

    /// A literal assigned to an unannotated local whose type so far has it
    /// stays that literal (decision L61).
    #[test]
    fn assigned_known_literals() {
        // the lazily filled cache: `false` marks "none"
        strict_ok(
            "type W = { a: number }\nlocal cache: { [string]: W | false } = {}\nlocal function get(k: string): W?\n local w = cache[k]\n if w == nil then\n  w = false\n  cache[k] = w\n end\n return w or nil\nend\nprint(get)\n",
        );
        // a literal the local doesn't have still widens
        strict_err(
            "type W = { a: number }\nlocal cache: { [string]: W | false } = {}\nlocal function put(k: string)\n local w = cache[k]\n w = true\n cache[k] = w\nend\nprint(put)\n",
            "E2001",
        );
        strict_ok("local n = 1\nn = 2\nlocal s = \"x\"\ns = \"y\"\nprint(n + 1, s:upper())\n");
    }

    /// `select(n, …)` with a literal `n` gives the values from the n-th on
    /// (decision L54).
    #[test]
    fn literal_select() {
        strict_err(
            "local function three(): (number, string, boolean) return 1, \"a\", true end\nlocal s = select(2, three())\nlocal n: number = s\nprint(n)\n",
            "E2001",
        );
        strict_ok(
            "local function three(): (number, string, boolean) return 1, \"a\", true end\nlocal b: boolean = select(3, three())\nlocal c = select(\"#\", three())\nprint(b, c)\n",
        );
    }

    /// An `unknown` narrowed to a function or a table stays `unknown`
    /// inside; an `any` keeps the plain `function` and `table` (decision L75).
    #[test]
    fn narrowed_unknown_stays_unknown_inside() {
        strict_err(
            "local function f(v: unknown) if type(v) == \"table\" then local s: string = v.name print(s) end end\nf(1)\n",
            "E2001",
        );
        strict_err(
            "local function f(v: unknown) if type(v) == \"function\" then local s: string = v() print(s) end end\nf(1)\n",
            "E2001",
        );
        strict_ok(
            "local function f(v: unknown) if type(v) == \"table\" then for k, x in pairs(v) do print(tostring(k), tostring(x)) end end end\nf(1)\n",
        );
        strict_ok(
            "local function f(v: unknown) if type(v) == \"function\" then local ok, r = pcall(v, 1, \"a\") print(ok, tostring(r)) end end\nf(1)\n",
        );
        strict_ok(
            "local function f(v: unknown) if type(v) == \"table\" then local n = v.n if type(n) == \"number\" then print(n + 1) end end end\nf(1)\n",
        );
        // an `any` keeps the plain types
        strict_ok(
            "local function f(v: any) if type(v) == \"table\" then local s: string = v.name print(s) end end\nf(1)\n",
        );
    }

    /// A callback's returns bind a type parameter nothing else binds
    /// (decision L55).
    #[test]
    fn callback_result_binding() {
        strict_err(
            "local function call<R>(f: () -> R): R return f() end\nlocal x = call(function() return \"x\" end)\nlocal n: number = x\nprint(n)\n",
            "E2001",
        );
        strict_err(
            "local ok, h = pcall(function() return \"x\" end)\nlocal e: number = h\nprint(ok, e)\n",
            "E2001",
        );
        strict_err(
            "local ok, v = pcall(function(a: number) return a * 2 end, 3)\nlocal e: string = v\nprint(ok, e)\n",
            "E2001",
        );
        strict_ok(
            "local ok, h = pcall(function(): string return \"x\" end)\nlocal e: string = h\nprint(ok, e)\n",
        );
        // a function that only raises: what it raised, `unknown` (L75)
        strict_err(
            "local ok, m = pcall(error, \"x\")\nlocal s: number = m\nprint(ok, s)\n",
            "E2001",
        );
        strict_err(
            "local ok, m = pcall(function() error(\"x\") end)\nlocal s: number = m\nprint(ok, s)\n",
            "E2001",
        );
        strict_ok("local ok, m = pcall(error, \"x\")\nprint(ok, tostring(m))\n");
    }

    /// A record read with a key of its field names gives those fields'
    /// types (decision L53).
    #[test]
    fn literal_key_reads() {
        strict_err(
            "type C = { a: number, b: number }\nlocal c: C = { a = 1, b = 2 }\nlocal function f(k: keyof C): string return c[k] end\nprint(f)\n",
            "E2001",
        );
        strict_ok(
            "type C = { a: number, b: number }\nlocal c: C = { a = 1, b = 2 }\nlocal function f(k: \"a\" | \"b\"): number return c[k] end\nprint(f)\n",
        );
    }

    /// A constrained type parameter's members are its constraint's (decision
    /// T279); an unconstrained one's are `any`.
    #[test]
    fn type_parameter_members() {
        strict_err(
            "class Shape\n function Area(): number return 1 end\nend\nlocal function f<S extends Shape>(s: S): string\n return s:Area()\nend\nprint(f)\n",
            "E2001",
        );
        strict_ok(
            "class Shape\n function Area(): number return 1 end\nend\nlocal function f<S extends Shape>(s: S): number\n return s:Area()\nend\nprint(f)\n",
        );
        strict_err(
            "class Shape\n size: number = 1\nend\nclass Box<S extends Shape>\n items: S[] = {}\n function First(): string return self.items[1].size end\nend\nprint(Box)\n",
            "E2001",
        );
        strict_ok("local function g<T>(t: T): number\n return t.n\nend\nprint(g)\n");
    }

    /// E2015 on a possibly secret value points to the secret predicates.
    #[test]
    fn cast_help() {
        let d = check_source(
            "local function f(v: string | Secret<string>)\n local s = v :: string?\n print(s)\nend\nprint(f)\n",
            Mode::Strict,
        );
        assert!(
            d.iter().any(|x| x.code == Some("E2015")
                && x.message
                    .contains("`Secret<string>` does not fit `string?`")
                && x.help.as_deref().is_some_and(
                    |h| h.contains("canaccessvalue") && h.contains(":: any :: string?")
                )),
            "{d:#?}"
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
        module_diagnostics(modules, mode)
            .iter()
            .filter(|x| x.is_error())
            .map(|x| format!("{}: {}", x.code.unwrap_or("-"), x.message))
            .collect()
    }

    /// [`check_modules`]'s diagnostics, warnings too.
    fn module_diagnostics(modules: &[(&str, &str)], mode: Mode) -> Vec<Diagnostic> {
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
        d
    }

    /// A predicate branch no member of its argument reaches is unreachable:
    /// the argument is `never` there and nothing in it is reported (spec
    /// §2.2.8), for `v is T` predicates and `canaccessvalue`.
    #[test]
    fn predicate_branches_without_members_are_unreachable() {
        strict_ok(
            "local function f(c: string | Secret<string> | nil)\n    if not issecretvalue(c) and c then\n        print(c:lower())\n        return\n    end\n    if not issecretvalue(c) and c then\n        print(c:lower())\n        local s: number = \"x\"\n        print(s)\n    end\nend\nf(nil)\n",
        );
        strict_ok(
            "local function g(v: string | Secret<string>)\n    if issecretvalue(v) then return end\n    if issecretvalue(v) then\n        print(v:upper())\n    end\n    if not canaccessvalue(v) then\n        print(v:upper())\n    end\n    print(v:lower())\nend\ng(\"a\")\n",
        );
        strict_ok(
            "local function isNum(x: unknown): x is number return type(x) == \"number\" end\nlocal function h(x: number | string): string\n    if isNum(x) then return \"n\" end\n    if isNum(x) then\n        local s: string = x\n        return s\n    elseif x == \"a\" then\n        return x\n    else\n        return x\n    end\nend\nprint(h(1))\n",
        );
        // The `else` of a predicate every member fits.
        strict_ok(
            "local function isNum(x: unknown): x is number return type(x) == \"number\" end\nlocal function k(x: number): number\n    if isNum(x) then\n        return x\n    else\n        local s: string = x\n        return #s\n    end\nend\nprint(k(1))\n",
        );
        // A reachable branch is still checked, and a downcast still narrows.
        strict_err(
            "local function isNum(x: unknown): x is number return type(x) == \"number\" end\nlocal function h(x: number | string)\n    if isNum(x) then\n        local s: string = x\n        print(s)\n    end\nend\nh(1)\n",
            "E2001",
        );
        strict_ok(
            "class A\nend\nclass B extends A\n    function only(): number return 1 end\nend\nlocal function isB(a: A): a is B return a ~= nil end\nlocal function c(a: A): number\n    if isB(a) then return a:only() end\n    return 0\nend\nprint(c(A.new()))\n",
        );
        // `any` may be either.
        strict_err(
            "local function isNum(x: unknown): x is number return type(x) == \"number\" end\nlocal function a(x: any)\n    if isNum(x) then return end\n    local s: number = \"x\"\n    print(s, x)\nend\na(1)\n",
            "E2001",
        );
    }

    /// `type X = typeof(v)` is the type of `v` where the alias stands, in
    /// later annotations of its file and for importers, by name or through
    /// a type-only import (spec §2.2.7).
    /// A class field typed by another module's `typeof` alias (a record of
    /// `typeof(UnitHealth)` and the like) reads the alias's type in the
    /// class's methods, not `any` (decision L76).
    #[test]
    fn typeof_alias_of_another_module_in_classes() {
        let api = "local function health(u: string): number return #u end\nexport type Api = { health: typeof(health) }\n";
        let errs = check_modules(
            &[
                ("api.wowlua", api),
                (
                    "main.wowlua",
                    "import type Api from \"./api\"\nclass Plate\n api: Api\n constructor(api: Api) self.api = api end\n function Read(): string return self.api.health(\"player\") end\nend\nprint(Plate)\n",
                ),
            ],
            Mode::Strict,
        );
        assert!(errs.len() == 1 && errs[0].starts_with("E2001"), "{errs:#?}");
        let errs = check_modules(
            &[
                ("api.wowlua", api),
                (
                    "main.wowlua",
                    "import type Api from \"./api\"\nclass Plate\n api: Api\n constructor(api: Api) self.api = api end\n function Read(): number return self.api.health(\"player\") end\nend\nprint(Plate)\n",
                ),
            ],
            Mode::Strict,
        );
        assert!(errs.is_empty(), "{errs:#?}");
    }

    #[test]
    fn callback_parameters_take_the_rest_type() {
        strict_err(
            "local t = \"a=b\"\nlocal s = t:gsub(\"(%w)=(%w)\", function(k, v) local n: number = v; return k .. n end)\nprint(s)\n",
            "E2001",
        );
        strict_ok(
            "local t = \"a=b\"\nlocal s = t:gsub(\"(%w)=(%w)\", function(k, v) return k:upper() .. v end)\nprint(s)\n",
        );
    }

    #[test]
    fn method_through_an_indexer_is_its_value() {
        let t = "type R = { Get: (self: R) -> number, [string]: unknown }\n";
        strict_err(
            &format!("{t}local function f(r: R) r:Setting(\"x\") end\nprint(f)\n"),
            "E2004",
        );
        strict_ok(&format!(
            "{t}local function f(r: R): number return r:Get() end\nprint(f)\n"
        ));
        strict_ok(
            "type H = (self: S, n: number) -> number\ntype S = { [string]: H }\nlocal function f(s: S): number return s:Any(1) end\nprint(f)\n",
        );
    }

    #[test]
    fn typeof_aliases() {
        strict_err(
            "local defaults = { scale = 1, locked = false }\ntype S = typeof(defaults)\nlocal s: S = defaults\nlocal x: string = s\n",
            "E2001",
        );
        strict_ok(
            "local defaults = { scale = 1, locked = false }\ntype S = typeof(defaults)\nlocal s: S = { scale = 2, locked = true }\nlocal n: number = s.scale\nprint(n)\n",
        );
        let conf = "local defaults = { scale = 1, locked = false }\nexport type Settings = typeof(defaults)\nexport function make(): Settings return { scale = 2, locked = true } end\n";
        let errs = check_modules(
            &[
                ("conf.wowlua", conf),
                (
                    "main.wowlua",
                    "import type Settings from \"./conf\"\nlocal a: Settings = { scale = 1, locked = \"no\" }\nlocal b: Settings = { scale = 1, locked = true }\nprint(a, b)\n",
                ),
            ],
            Mode::Strict,
        );
        assert!(
            !errs.is_empty()
                && errs
                    .iter()
                    .all(|e| e.starts_with("E2001") && e.contains("no")),
            "{errs:#?}"
        );
        // The importer comes first in source order; the type-only import
        // still orders the check.
        let errs = check_modules(
            &[
                (
                    "a.wowlua",
                    "import type Settings from \"./conf\"\nlocal a: Settings = { scale = \"x\", locked = true }\nprint(a)\n",
                ),
                ("conf.wowlua", conf),
            ],
            Mode::Strict,
        );
        assert!(
            !errs.is_empty() && errs.iter().all(|e| e.starts_with("E2001")),
            "{errs:#?}"
        );
    }

    /// `errors.tryValue`'s signature: the result is the function's own first
    /// result, optional; the arguments are checked against its parameters
    /// (decision S43).
    #[test]
    fn try_value_keeps_the_result_type() {
        let pre = "local function tryValue<A..., R>(fn: ((A...) -> R)?, ...: A...): R?\n    if type(fn) ~= \"function\" then return nil end\n    local ok, value = pcall(fn, ...)\n    if ok then return value end\n    return nil\nend\nlocal function lookup(id: number): { name: string }? return id == 1 and { name = \"x\" } or nil end\nlocal function two(): (number, string) return 1, \"x\" end\n";
        strict_ok(&format!(
            "{pre}local info = tryValue(lookup, 1)\nlocal n: string? = info and info.name\nlocal m: number? = tryValue(two)\nprint(n, m)\n"
        ));
        strict_err(
            &format!("{pre}local bad: number? = tryValue(lookup, 1)\nprint(bad)\n"),
            "E2001",
        );
        strict_err(
            &format!("{pre}local info = tryValue(lookup, \"1\")\nprint(info)\n"),
            "E2001",
        );
    }

    /// A class extends a value: a class `import type` brings is a type only,
    /// with nothing to extend at run time (E3008); imported as a value too,
    /// it is fine.
    #[test]
    fn class_extends_a_type_only_import() {
        let base = "export class Slot\n    size: number = 1\nend\n";
        let errs = check_modules(
            &[
                ("slot.wowlua", base),
                (
                    "main.wowlua",
                    "import type Slot from \"./slot\"\nclass Wide extends Slot\nend\nprint(Wide)\n",
                ),
            ],
            Mode::Strict,
        );
        assert!(
            errs.len() == 1
                && errs[0].starts_with("E3008")
                && errs[0].contains("`Wide` extends `Slot`"),
            "{errs:#?}"
        );
        let errs = check_modules(
            &[
                ("slot.wowlua", base),
                (
                    "main.wowlua",
                    "import Slot from \"./slot\"\nimport type Slot as S from \"./slot\"\nclass Wide extends Slot\nend\nlocal w: S = Wide.new()\nprint(w)\n",
                ),
            ],
            Mode::Strict,
        );
        assert!(errs.is_empty(), "{errs:#?}");
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
        // `readonly` works on them as on named fields.
        strict_ok(
            "type R = { readonly [\"end\"]: number, readonly [\"not\"]?: boolean }\nlocal r: R = { [\"end\"] = 1 }\nlocal e: number = r[\"end\"]\nprint(e)\n",
        );
        strict_err(
            "type R = { readonly [\"end\"]: number }\nlocal r: R = { [\"end\"] = 1 }\nr[\"end\"] = 2\n",
            "E2010",
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
        // `table` is a map of `any` values with any key type, not a typed
        // map (spec §2.6).
        strict_ok(
            "local t: table = {}\nlocal m: { [string]: any } = t\nlocal n: { [number]: any } = t\nprint(m, n)\n",
        );
        strict_ok(
            "local function f(x: any)\n    if type(x) == \"table\" then\n        local m: { [string]: any } = x\n        print(m)\n    end\nend\nf({})\n",
        );
        strict_err(
            "local t: table = {}\nlocal m: { [string]: number } = t\nprint(m)\n",
            "E2001",
        );
        strict_err(
            "local t: table = {}\nlocal m: { [string]: unknown } = t\nprint(m)\n",
            "E2001",
        );
    }

    /// An array value fits a record whose named fields are all optional and
    /// whose numeric indexer takes its elements (spec §2.6), alone or in an
    /// intersection with an array type.
    #[test]
    fn arrays_for_records_with_optional_fields() {
        strict_ok(
            "type By = { [number]: any, talent?: boolean, what?: string }\nlocal xs: string[] = { \"MAGE\" }\nlocal x: By = xs\nprint(x)\n",
        );
        strict_ok(
            "type Art = any[] & { jewel?: boolean }\nlocal a: Art = { \"x\", 1, 2 }\nlocal b: Art = { \"y\", jewel = true }\nlocal xs: any[] = {}\nlocal c: Art = xs\nprint(a, b, c)\n",
        );
        strict_ok(
            "type Ns = { [number]: string, n?: number }\nlocal xs: string[] = { \"a\" }\nlocal ns: Ns = xs\nprint(ns)\n",
        );
        // A required named field is still required.
        strict_err(
            "type Ns = { [number]: string, n: number }\nlocal xs: string[] = { \"a\" }\nlocal ns: Ns = xs\nprint(ns)\n",
            "E2001",
        );
        // The indexer must take the elements.
        strict_err(
            "type Ns = { [number]: string, n?: number }\nlocal xs: number[] = { 1 }\nlocal ns: Ns = xs\nprint(ns)\n",
            "E2001",
        );
        strict_err(
            "type Ks = { [string]: string, n?: number }\nlocal xs: string[] = { \"a\" }\nlocal ks: Ks = xs\nprint(ks)\n",
            "E2001",
        );
        // Without an indexer only beside an array type in an intersection.
        strict_err(
            "local xs: string[] = { \"a\" }\nlocal r: { name?: string } = xs\nprint(r)\n",
            "E2001",
        );
    }
}
