//! The tests of a test file, for the editor's test explorer
//! (`wowlua/tests`): each function decorated with `wowlua:test`'s `@test`,
//! its name (`@test("…")`'s, else the function's) and place, read from the
//! file's syntax (spec §16.2). What only a run knows (each case of a
//! `@cases` test, the tests a `describe` defines) comes with the run's
//! results.

use crate::text;
use serde_json::{Value as Json, json};
use std::collections::HashMap;
use wowlua_span::SourceDb;
use wowlua_syntax::view::{ExprKind, ImportClause, StmtKind};

/// The tests of `src`: `{ name, range, skip, cases }` each.
pub fn tests(src: &str) -> Vec<Json> {
    let mut db = SourceDb::new();
    let file = db.add("tests", String::new());
    let parse = wowlua_syntax::parse(src.as_bytes(), file);
    let block = parse.view().block();
    // `wowlua:test`'s exports by local name, and the namespaces it is
    // imported as (`import * as t`)
    let mut names: HashMap<String, String> = HashMap::new();
    let mut namespaces: Vec<String> = Vec::new();
    for s in block.stmts() {
        let StmtKind::Import(import) = s.kind() else {
            continue;
        };
        if &*import.from().value() != b"wowlua:test" {
            continue;
        }
        match import.clause() {
            ImportClause::Names(specs) => {
                for spec in specs {
                    let local = spec.alias.as_ref().unwrap_or(&spec.name);
                    names.insert(local.text().to_string(), spec.name.text().to_string());
                }
            }
            ImportClause::Namespace(n) => namespaces.push(n.text().to_string()),
            ImportClause::SideEffect => {}
        }
    }
    let mut out = Vec::new();
    for s in block.stmts() {
        let (function, at) = match s.kind() {
            StmtKind::LocalFunction { name, .. } => (name.text().to_string(), name.span()),
            StmtKind::Function { name, .. } => {
                let path: Vec<&str> = name.path().iter().map(|n| n.text()).collect();
                (path.join("."), name.span())
            }
            _ => continue,
        };
        let mut test: Option<Option<String>> = None;
        let mut skip = false;
        let mut cases = false;
        let mut compile_only = false;
        for d in s.decorators() {
            let path: Vec<&str> = d.path().iter().map(|n| n.text()).collect();
            // the export this decorator is
            let export = match path.as_slice() {
                [one] => names.get(*one).map(String::as_str),
                [ns, member] if namespaces.iter().any(|n| n == ns) => Some(*member),
                _ => None,
            };
            match export {
                Some("test") => {
                    let named = d.args().and_then(|a| {
                        a.exprs().iter().next().and_then(|e| match e.kind() {
                            ExprKind::String(s) => {
                                Some(String::from_utf8_lossy(&s.value()).into_owned())
                            }
                            _ => None,
                        })
                    });
                    test = Some(named);
                }
                Some("skip") => skip = true,
                Some("cases") => cases = true,
                Some("expectError") => compile_only = true,
                _ => {}
            }
        }
        let Some(named) = test else {
            continue;
        };
        if compile_only {
            continue;
        }
        let span = s.span().range();
        let name_at = at.range();
        out.push(json!({
            "name": named.unwrap_or(function),
            "range": text::range(src, span.start, span.end),
            "selectionRange": text::range(src, name_at.start, name_at.end),
            "skip": skip,
            "cases": cases,
        }));
    }
    out
}

#[cfg(test)]
mod tests_of_files {
    use super::tests;

    #[test]
    fn a_files_tests_by_name_and_place() {
        let src = "import test, skip, cases as each from \"wowlua:test\"\nimport * as t from \"wowlua:test\"\n\n@test(\"counts clicks\")\nlocal function clicks()\nend\n\n@test\nlocal function bare()\nend\n\n@skip(\"later\")\n@test\nlocal function later()\nend\n\n@each({ 1, 2 })\n@t.test(\"adds\")\nlocal function adds(n: number)\nend\n\nlocal function helper()\nend\n";
        let found = tests(src);
        let names: Vec<&str> = found.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(names, vec!["counts clicks", "bare", "later", "adds"]);
        assert_eq!(found[0]["range"]["start"]["line"], 3);
        assert_eq!(found[2]["skip"], true);
        assert_eq!(found[3]["cases"], true);
    }

    #[test]
    fn a_test_decorator_of_another_module_is_none() {
        let src = "import test from \"./mine\"\n\n@test(\"x\")\nlocal function f()\nend\n";
        assert!(tests(src).is_empty());
    }
}
