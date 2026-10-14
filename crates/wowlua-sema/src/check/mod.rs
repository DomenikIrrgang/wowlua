//! The type checker (spec chapter 2, compiler §13.6): declarations, type
//! resolution, assignability and the bidirectional checking of function
//! bodies with flow-sensitive narrowing.

pub mod body;
pub mod env;
pub mod expr;
pub mod flavors;
pub mod narrow;
pub mod query;
pub mod runtime;
pub mod subst;
pub mod types;

use crate::{Analysis, Export, FileInput, FileKind, Target};
use body::{Checker, FileInfo};
use query::Mode;
use std::collections::{HashMap, HashSet};
use types::{Field, Record, Ty, TypeId, Types};
use wowlua_diagnostics::Diagnostic;
use wowlua_syntax::ast::{ExprKind, ImportClause, StmtKind};

/// How one file is checked (spec §2.1).
#[derive(Clone, Copy, Debug)]
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
    check_package_full(files, analysis, settings, extra_globals, shared).diagnostics
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

/// Like [`check_package`], with the type information lowering uses.
pub fn check_package_full(
    files: &[FileInput],
    analysis: &Analysis,
    settings: &[Option<FileSettings>],
    extra_globals: &HashSet<String>,
    shared: Option<&flavors::SharedFlavors>,
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
                enums: HashMap::new(),
                declares,
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
        for s in &parse.chunk.block.stmts {
            let StmtKind::Import(import) = &s.kind else {
                continue;
            };
            let Some(resolved) = info.imports.iter().find(|r| r.start == s.span.start) else {
                continue;
            };
            let Target::Module(m) = resolved.target else {
                continue;
            };
            match &import.clause {
                ImportClause::Names(specs) => {
                    for spec in specs {
                        let local = spec.alias.as_ref().unwrap_or(&spec.name);
                        imports.push((local.text.clone(), m, spec.name.text.clone()));
                    }
                }
                ImportClause::Namespace(n) => namespaces.push((n.text.clone(), m)),
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
        });
    }
    let mut env = env::Env::build(&sources);
    let mut diagnostics = std::mem::take(&mut env.diagnostics);
    env.callbacks_registry = files
        .iter()
        .position(|f| f.src_rel == format!("{}callbacks.wowlua", crate::STDLIB_PREFIX))
        .and_then(|i| env.module_types.get(&i))
        .and_then(|t| t.get("Registry"))
        .copied();
    // Shared mode: APIs only some flavors declare are known (as `any`), so
    // that their uses report E2030.
    if let Some(shared) = shared {
        for path in shared.availability.keys() {
            if path.starts_with("event:") {
                continue;
            }
            match path.rsplit_once('.') {
                None => {
                    env.globals
                        .entry(path.as_str().into())
                        .or_insert(Types::ANY);
                }
                Some((ns, member)) => {
                    if let Some(&d) = env.global_types.get(ns)
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
    let mut extra_globals = extra_globals.clone();
    let mut assigned_globals = HashSet::new();
    let markers = crate::usage::Markers::default();
    for (i, f) in files.iter().enumerate() {
        let Some(parse) = f.parse else { continue };
        if f.kind == FileKind::Wowlua {
            global_decorated(&parse.chunk.block, &mut extra_globals);
        }
        if settings.get(i).copied().flatten().is_some() {
            for d in crate::usage::file_usage(parse, &markers).defs {
                if let crate::usage::Key::Global(name) = d.key {
                    assigned_globals.insert(name);
                }
            }
            // `CreateFrame(type, "Name", …)` creates the global `Name`.
            wowlua_syntax::visit::walk_block(&parse.chunk.block, &mut |e| {
                if let ExprKind::Call { callee, args } = &e.kind
                    && matches!(&callee.kind, ExprKind::Name(n) if n.text.as_ref() == "CreateFrame")
                    && let Some(ExprKind::String(lit)) = args.exprs().get(1).map(|a| &a.kind)
                {
                    assigned_globals.insert(String::from_utf8_lossy(&lit.value).into_owned());
                }
            });
        }
    }
    let extra_globals = &extra_globals;
    let assigned_globals = &assigned_globals;

    // Modules before their importers, so export types are known.
    let order = check_order(files.len(), &analysis.edges);
    let mut exports: HashMap<usize, HashMap<Box<str>, TypeId>> = HashMap::new();
    for i in order {
        let (Some(settings), Some(parse)) = (settings.get(i).copied().flatten(), files[i].parse)
        else {
            continue;
        };
        if !matches!(files[i].kind, FileKind::Wowlua | FileKind::Lua) {
            continue;
        }
        let imports = import_types(&mut env, files, analysis, i, &exports);
        let info = FileInfo {
            intrinsics: crate::stdlib::Intrinsics::of(analysis, i, parse),
            index: i,
            mode: settings.mode,
            pure_lua: settings.pure_lua,
            extra_globals,
            assigned_globals,
            imports,
            is_module: analysis.modules[i].as_ref().is_some_and(|m| m.is_module),
            shared,
            meta_flavor: analysis.modules[i]
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
        let mut checker = Checker::new(&mut env, &info, parse);
        checker.check_file(parse);
        // `typeName(x)` arguments (the callee's name decides; lowering only
        // uses entries of `wowlua:debug` intrinsics).
        wowlua_syntax::visit::walk_block(&parse.chunk.block, &mut |e| {
            if let ExprKind::Call { callee, args } = &e.kind
                && matches!(&callee.kind, ExprKind::Name(n) if n.text.as_ref() == "typeName")
                    | matches!(&callee.kind, ExprKind::Field { name, .. } if name.text.as_ref() == "typeName")
                && let Some(arg) = args.exprs().first()
                && let Some(&t) = checker.expr_types.get(&(arg.span.start, arg.span.end))
            {
                type_names.insert(arg.span, checker.env.display(t));
            }
        });
        value_checks.extend(std::mem::take(&mut checker.value_checks));
        private_methods.extend(std::mem::take(&mut checker.private_methods));
        // `@global` declarations hand their classes to other addons: the
        // class itself, the types of its values, the types its signature
        // names.
        for st in &parse.chunk.block.stmts {
            if !st.decorators.iter().any(|d| d.is_named("global")) {
                continue;
            }
            let inner = match &st.kind {
                StmtKind::Export(i) => i.as_ref(),
                _ => st,
            };
            let mut names: Vec<&str> = Vec::new();
            match &inner.kind {
                StmtKind::Class(c) => names.push(&c.name.text),
                StmtKind::Local { names: ns, values } => {
                    for v in values {
                        if let Some(&t) = checker.expr_types.get(&(v.span.start, v.span.end)) {
                            checker.note_global_classes(t);
                        }
                    }
                    for n in ns {
                        if let Some(t) = &n.ty {
                            type_refs(t, &mut names);
                        }
                    }
                }
                StmtKind::LocalFunction { func, .. } | StmtKind::Function { func, .. } => {
                    for p in &func.params {
                        if let Some(t) = &p.ty {
                            type_refs(t, &mut names);
                        }
                    }
                    match &func.ret {
                        Some(wowlua_syntax::ast::RetType::Single(t)) => type_refs(t, &mut names),
                        Some(wowlua_syntax::ast::RetType::List { types, .. }) => {
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
                    .or_else(|| checker.env.global_types.get(name))
                    .copied();
                if let Some(d) = found {
                    global_classes.insert(d);
                }
            }
        }
        global_classes.extend(std::mem::take(&mut checker.global_classes));
        let mut file_exports = std::mem::take(&mut checker.exports);
        diagnostics.append(&mut checker.diagnostics);
        diagnostics.append(&mut env.diagnostics);
        // A standard-library module's declarations give its exports as
        // callers see them (the compiler fills in some arguments).
        // (A declaration typed `any` only binds an intrinsic.)
        if let Some(declared) = env.declared_exports.get(&i) {
            file_exports.extend(
                declared
                    .iter()
                    .filter(|(_, t)| **t != Types::ANY)
                    .map(|(k, &t)| (k.clone(), t)),
            );
        }
        exports.insert(i, file_exports);
    }
    private_methods.extend(env.renamable_defs.iter().copied());
    // Classes reachable from globals, and every class of their hierarchies,
    // keep their method names (`rename-private`).
    let mut reachable = global_classes;
    loop {
        let before = reachable.len();
        for (i, def) in env.defs.iter().enumerate() {
            let id = DefId(i as u32);
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
fn type_refs<'a>(t: &'a wowlua_syntax::ast::Type, out: &mut Vec<&'a str>) {
    use wowlua_syntax::ast::TypeKind;
    match &t.kind {
        TypeKind::Ref { path, args } => {
            if let Some(last) = path.last() {
                out.push(&last.text);
            }
            for a in args {
                if let wowlua_syntax::ast::TypeArg::Type(t) = a {
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

/// Names of declarations decorated with `@global`.
fn global_decorated(block: &wowlua_syntax::ast::Block, out: &mut HashSet<String>) {
    use wowlua_syntax::ast::ExprKind;
    for s in &block.stmts {
        let Some(d) = s.decorators.iter().find(|d| d.is_named("global")) else {
            continue;
        };
        let explicit = d
            .args
            .as_ref()
            .and_then(|a| match a.exprs().first().map(|e| &e.kind) {
                Some(ExprKind::String(lit)) => {
                    Some(String::from_utf8_lossy(&lit.value).into_owned())
                }
                _ => None,
            });
        if let Some(name) = explicit {
            out.insert(name);
            continue;
        }
        let inner = match &s.kind {
            StmtKind::Export(i) => &i.kind,
            k => k,
        };
        match inner {
            StmtKind::Local { names, .. } => {
                out.extend(names.iter().map(|n| n.name.text.to_string()))
            }
            StmtKind::LocalFunction { name, .. } | StmtKind::Const { name, .. } => {
                out.insert(name.text.to_string());
            }
            StmtKind::Function { name, .. } => {
                out.insert(
                    name.method
                        .as_ref()
                        .unwrap_or_else(|| name.path.last().expect("a name"))
                        .text
                        .to_string(),
                );
            }
            StmtKind::Class(c) => {
                out.insert(c.name.text.to_string());
            }
            StmtKind::Enum(e) => {
                out.insert(e.name.text.to_string());
            }
            _ => {}
        }
    }
}

/// A topological order of the files over import edges, source order for
/// ties and cycles.
fn check_order(n: usize, edges: &[(usize, usize)]) -> Vec<usize> {
    let rank: Vec<usize> = (0..n).collect();
    match crate::order::stable_topo(n, &rank, edges) {
        Ok(order) => order,
        Err(_) => (0..n).collect(),
    }
}

/// The types of a file's imported bindings.
fn import_types(
    env: &mut env::Env,
    files: &[FileInput],
    analysis: &Analysis,
    file: usize,
    exports: &HashMap<usize, HashMap<Box<str>, TypeId>>,
) -> HashMap<Box<str>, TypeId> {
    let mut out = HashMap::new();
    let (Some(parse), Some(info)) = (files[file].parse, &analysis.modules[file]) else {
        return out;
    };
    for s in &parse.chunk.block.stmts {
        let StmtKind::Import(import) = &s.kind else {
            continue;
        };
        if import.type_only {
            continue;
        }
        let Some(resolved) = info.imports.iter().find(|r| r.start == s.span.start) else {
            continue;
        };
        let module = match resolved.target {
            Target::Module(m) => Some(m),
            _ => None,
        };
        let export_type = |env: &mut env::Env, name: &str| -> TypeId {
            let Some(m) = module else { return Types::ANY };
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
        match &import.clause {
            ImportClause::Names(specs) => {
                for spec in specs {
                    let local = spec.alias.as_ref().unwrap_or(&spec.name);
                    let t = export_type(env, &spec.name.text);
                    out.insert(local.text.clone(), t);
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
                out.insert(n.text.clone(), t);
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
            })
            .collect();
        inputs.push(FileInput {
            src_rel: "t.wowlua".into(),
            kind: FileKind::Wowlua,
            parse: Some(&main),
            xml: None,
            meta: None,
            main_of: None,
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
}
