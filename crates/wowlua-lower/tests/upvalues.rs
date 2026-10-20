//! The upvalue limit (spec §5.5): a function that would capture more than
//! 60 imports reads the excess through the module registry, and runs.

use std::collections::HashMap;
use wowlua_lower::{Context, ImportContext};
use wowlua_luair::{EmitOptions, emit};
use wowlua_span::SourceDb;

/// A module importing `n` modules (`namespace`: `import * as mI`, else
/// `import command as cI`) with one function using every import.
fn lowered(n: usize, namespace: bool, lazy: bool) -> String {
    let mut src = String::new();
    let mut imports = HashMap::new();
    for i in 1..=n {
        imports.insert(
            src.len() as u32,
            ImportContext::Module {
                id: format!("m/M{i}"),
                lazy,
                exports: HashMap::new(),
                attrs: HashMap::new(),
                intrinsics: HashMap::new(),
            },
        );
        if namespace {
            src.push_str(&format!("import * as m{i} from \"/m/M{i}\"\n"));
        } else {
            src.push_str(&format!("import command as c{i} from \"/m/M{i}\"\n"));
        }
    }
    src.push_str("export function run(sub)\n");
    for i in 1..=n {
        let call = if namespace {
            format!("m{i}.command()")
        } else {
            format!("c{i}()")
        };
        src.push_str(&format!("    if sub == \"{i}\" then return {call} end\n"));
    }
    src.push_str("end\n");
    let ctx = Context {
        module: Some("main".to_string()),
        imports,
        ..Context::default()
    };
    let mut db = SourceDb::new();
    let file = db.add("t.wowlua", src.clone());
    let parse = wowlua_syntax::parse(src.as_bytes(), file);
    assert!(!parse.has_errors(), "{:?}", parse.diagnostics);
    let lowered = match wowlua_lower::lower_module(&parse, src.as_bytes(), &ctx) {
        Ok(l) => l,
        Err(e) => panic!("{:?}", e.iter().map(|d| &d.message).collect::<Vec<_>>()),
    };
    String::from_utf8(emit(&lowered.chunk, &EmitOptions::default()).text).unwrap()
}

/// Loads `code` with a stand-in runtime of `n` modules and calls `run` for
/// every module: what each returned.
fn run(code: &str, n: usize) -> Vec<String> {
    let lua = mlua::Lua::new();
    let runtime: mlua::Table = lua
        .load(
            r#"
            local modules = {}
            for i = 1, 100 do
                local id = "m/M" .. i
                modules[id] = { command = function() return id end }
            end
            local wl = { modules = modules }
            function wl.module(id) modules[id] = modules[id] or {} return modules[id] end
            function wl.require(id) return modules[id] end
            return { __wowlua = wl }
            "#,
        )
        .eval()
        .unwrap();
    let chunk = lua.load(code).into_function().unwrap();
    chunk.call::<()>(("X", runtime.clone())).unwrap();
    let main: mlua::Table = runtime
        .get::<mlua::Table>("__wowlua")
        .unwrap()
        .get::<mlua::Table>("modules")
        .unwrap()
        .get("main")
        .unwrap();
    let f: mlua::Function = main.get("run").unwrap();
    (1..=n)
        .map(|i| f.call::<String>(i.to_string()).unwrap())
        .collect()
}

#[test]
fn excess_imports_are_read_through_the_registry() {
    for (namespace, lazy) in [(true, false), (false, false), (false, true), (true, true)] {
        let code = lowered(70, namespace, lazy);
        assert!(
            code.contains("__wl.modules[\"m/M70\"]"),
            "namespace {namespace}, lazy {lazy}:\n{code}"
        );
        // The first imports stay locals.
        assert!(!code.contains("__wl.modules[\"m/M1\"]"), "{code}");
        let results = run(&code, 70);
        let expected: Vec<String> = (1..=70).map(|i| format!("m/M{i}")).collect();
        assert_eq!(results, expected);
    }
    // Under the limit, every import stays a local.
    let code = lowered(40, true, false);
    assert!(!code.contains("__wl.modules"), "{code}");
}
