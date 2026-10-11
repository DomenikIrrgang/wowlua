//! The type checker (spec chapter 2, compiler §13.6): declarations, type
//! resolution, assignability and the bidirectional checking of function
//! bodies with flow-sensitive narrowing.

pub mod body;
pub mod env;
pub mod expr;
pub mod narrow;
pub mod query;
pub mod subst;
pub mod types;

use crate::{Analysis, Export, FileInput, FileKind, Target};
use body::{Checker, FileInfo};
use query::Mode;
use std::collections::{HashMap, HashSet};
use types::{Field, Record, Ty, TypeId, Types};
use wowlua_diagnostics::Diagnostic;
use wowlua_syntax::ast::{ImportClause, StmtKind};

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
) -> Vec<Diagnostic> {
    // Declaration sources: declaration files first, then modules.
    let mut sources: Vec<env::Source> = Vec::new();
    for (i, f) in files.iter().enumerate() {
        if f.kind == FileKind::Declaration
            && let Some(parse) = f.parse
        {
            sources.push(env::Source {
                index: i,
                parse,
                declaration: true,
                imports: Vec::new(),
                namespaces: Vec::new(),
                enums: HashMap::new(),
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
        });
    }
    let mut env = env::Env::build(&sources);
    let mut diagnostics = std::mem::take(&mut env.diagnostics);
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
            index: i,
            mode: settings.mode,
            pure_lua: settings.pure_lua,
            extra_globals,
            assigned_globals,
            imports,
            is_module: analysis.modules[i].as_ref().is_some_and(|m| m.is_module),
        };
        let mut checker = Checker::new(&mut env, &info, parse);
        checker.check_file(parse);
        let file_exports = std::mem::take(&mut checker.exports);
        diagnostics.append(&mut checker.diagnostics);
        diagnostics.append(&mut env.diagnostics);
        exports.insert(i, file_exports);
    }
    diagnostics
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
        let mut db = SourceDb::new();
        let mut parses = Vec::new();
        for decl in wowlua_stdlib_files() {
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
            })
            .collect();
        inputs.push(FileInput {
            src_rel: "t.wowlua".into(),
            kind: FileKind::Wowlua,
            parse: Some(&main),
            xml: None,
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
