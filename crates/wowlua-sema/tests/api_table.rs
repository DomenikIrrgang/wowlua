//! The WoW API's globals of a primitive type come from a table instead of
//! the parsed text: the type environment is the same as from the files in
//! `wowlua-api/data`.

use wowlua_sema::check::env::{Env, Source};
use wowlua_span::SourceDb;

const FLAVORS: &[&str] = &[
    "standard", "camelot", "mists", "cata", "wrath", "tbc", "vanilla",
];

fn pristine(name: &str) -> String {
    let path = format!(
        "{}/../wowlua-api/data/{name}.d.wowlua",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).unwrap()
}

/// What the environment says about globals, in a comparable form.
fn summary(files: &[(&str, String, wowlua_api::Globals)]) -> String {
    let mut db = SourceDb::new();
    let parses: Vec<wowlua_syntax::Parse> = files
        .iter()
        .map(|(path, text, _)| {
            let file = db.add(*path, text.clone());
            let parse = wowlua_syntax::parse(text.as_bytes(), file);
            assert!(!parse.has_errors(), "{path}");
            parse
        })
        .collect();
    let sources: Vec<Source> = parses
        .iter()
        .zip(files)
        .enumerate()
        .map(|(index, (parse, (_, _, globals)))| Source {
            index,
            parse,
            declaration: true,
            imports: Vec::new(),
            namespaces: Vec::new(),
            enums: Default::default(),
            declares: None,
            globals: *globals,
        })
        .collect();
    let env = Env::build(&sources);
    let mut globals: Vec<String> = env
        .globals
        .iter()
        .map(|(n, t)| format!("{n} {t:?} {}", env.display(*t)))
        .collect();
    globals.sort();
    let mut on_demand: Vec<String> = env
        .on_demand
        .iter()
        .map(|(n, a)| format!("{n} {a}"))
        .collect();
    on_demand.sort();
    let mut global_types: Vec<String> = env
        .global_types
        .iter()
        .map(|(n, d)| format!("{n} {d:?}"))
        .collect();
    global_types.sort();
    format!(
        "{} globals, {} on demand, {} types, {} defs\n{}\n{}\n{}\n{:?}",
        globals.len(),
        on_demand.len(),
        global_types.len(),
        env.defs.len(),
        globals.join("\n"),
        on_demand.join("\n"),
        global_types.join("\n"),
        env.diagnostics,
    )
}

#[test]
fn primitive_globals_build_the_same_env() {
    fn builtin(name: &'static str) -> (&'static str, String, wowlua_api::Globals) {
        let text = match name {
            "lua" => include_str!("../../wowlua-stdlib/decl/lua.d.wowlua"),
            _ => include_str!("../../wowlua-stdlib/decl/wow.d.wowlua"),
        };
        (name, text.to_string(), wowlua_api::Globals::default())
    }
    for flavor in FLAVORS {
        let [shared, own] = wowlua_api::declarations(flavor).unwrap();
        let tabled = vec![
            builtin("lua"),
            builtin("wow"),
            (shared.path, shared.text.to_string(), shared.globals),
            (own.path, own.text.to_string(), own.globals),
        ];
        let parsed = vec![
            builtin("lua"),
            builtin("wow"),
            (shared.path, pristine("shared"), Default::default()),
            (own.path, pristine(flavor), Default::default()),
        ];
        assert!(!own.globals.is_empty());
        let (a, b) = (summary(&tabled), summary(&parsed));
        assert!(a == b, "{flavor}: the environments differ");
        // Without the tables, the globals are missing.
        let mut untabled = tabled.clone();
        untabled[2].2 = Default::default();
        untabled[3].2 = Default::default();
        assert!(summary(&untabled) != a, "{flavor}");
    }
}
