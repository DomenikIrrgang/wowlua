//! What lowering reads of its context: a build reuses a file's lowering in
//! another flavor when the two contexts are the same apart from
//! `wowlua:meta` and the lowering did not read that.

use std::collections::HashMap;
use std::sync::Arc;
use wowlua_lower::{Context, ImportContext, Lowered};
use wowlua_luair::{EmitOptions, emit};
use wowlua_sema::Export;
use wowlua_sema::classes::EnumInfo;
use wowlua_sema::constant::Const;
use wowlua_sema::meta::{Meta, MetaValue};
use wowlua_span::SourceDb;

fn meta(flavor: &str) -> Option<Arc<Meta>> {
    Some(Arc::new(Meta {
        addon: MetaValue::Nil,
        build: MetaValue::Nil,
        config: MetaValue::Nil,
        features: MetaValue::Nil,
        flavor: Some(flavor.to_string()),
        assets: HashMap::new(),
    }))
}

fn lower(src: &str, ctx: &Context) -> Lowered {
    let mut db = SourceDb::new();
    let file = db.add("t.wowlua", src.to_string());
    let parse = wowlua_syntax::parse(src.as_bytes(), file);
    assert!(!parse.has_errors(), "{src}\n{:?}", parse.diagnostics);
    match wowlua_lower::lower_module(&parse, src.as_bytes(), ctx) {
        Ok(lowered) => lowered,
        Err(e) => panic!(
            "{src}\n{:?}",
            e.iter().map(|d| &d.message).collect::<Vec<_>>()
        ),
    }
}

fn code(lowered: &Lowered) -> String {
    String::from_utf8(emit(&lowered.chunk, &EmitOptions::default()).text).unwrap()
}

#[test]
fn lowering_records_whether_it_reads_meta() {
    let plain = Context {
        meta: meta("vanilla"),
        ..Context::default()
    };
    assert!(!lower("local x = 1\nprint(x)\n", &plain).reads_meta);
    let imports = Context {
        meta: meta("vanilla"),
        imports: HashMap::from([(0, ImportContext::Meta)]),
        ..Context::default()
    };
    let unused = "import flavor from \"wowlua:meta\"\nprint(1)\n";
    assert!(!lower(unused, &imports).reads_meta);
    let used = "import flavor from \"wowlua:meta\"\nprint(flavor)\n";
    let lowered = lower(used, &imports);
    assert!(lowered.reads_meta);
    assert!(
        code(&lowered).contains("print(\"vanilla\")"),
        "{}",
        code(&lowered)
    );
}

/// `0` and `-0` are equal numbers that are emitted differently, so contexts
/// that differ only by them are not the same; a NaN is the same as itself.
#[test]
fn contexts_compare_constants_bit_for_bit() {
    let with_export = |z: f64, flavor: &str| Context {
        meta: meta(flavor),
        file: "src/b.wowlua".to_string(),
        imports: HashMap::from([(
            0,
            ImportContext::Module {
                id: "a".to_string(),
                lazy: false,
                exports: HashMap::from([("Z".to_string(), Export::Const(Some(Const::Number(z))))]),
                attrs: HashMap::new(),
                intrinsics: HashMap::new(),
            },
        )]),
        ..Context::default()
    };
    let positive = with_export(0.0, "standard");
    assert!(positive.same_except_meta(&with_export(0.0, "vanilla")));
    assert!(!positive.same_except_meta(&with_export(-0.0, "vanilla")));
    assert!(!positive.same_except_meta(&with_export(1.0, "vanilla")));
    assert!(with_export(f64::NAN, "standard").same_except_meta(&with_export(f64::NAN, "vanilla")));
    let src = "import Z from \"./a\"\nprint(Z)\n";
    let a = lower(src, &positive);
    let b = lower(src, &with_export(-0.0, "vanilla"));
    assert!(!a.reads_meta && !b.reads_meta);
    assert_ne!(code(&a), code(&b));

    let with_enum = |member: f64| Context {
        enums: HashMap::from([(
            "E".to_string(),
            Arc::new(EnumInfo {
                name: "E".to_string(),
                members: vec![("A".to_string(), Const::Number(member))],
            }),
        )]),
        ..Context::default()
    };
    assert!(with_enum(0.0).same_except_meta(&with_enum(0.0)));
    assert!(!with_enum(0.0).same_except_meta(&with_enum(-0.0)));

    let debug = Context {
        debug: true,
        ..Context::default()
    };
    assert!(!debug.same_except_meta(&Context::default()));

    // What the checker found for the strict fast forms changes the code.
    let fast = Context {
        fast_forms: HashMap::from([((0, 1), wowlua_sema::check::fast::NEVER_FALSE)]),
        ..Context::default()
    };
    assert!(!fast.same_except_meta(&Context::default()));
}

/// The strict fast forms (spec §1.2.4, §1.4.1, §1.4.2) by what the checker
/// found: without facts (basic mode) the safe forms.
#[test]
fn fast_forms_follow_the_checker() {
    use wowlua_sema::check::fast::{NEVER_FALSE, STRING_OR_NUMBER};
    let src = "local a, b, s = ...\nprint(`{s}!`, a ?? b, a?.x)\n";
    // The span of the first `len` bytes of `text`.
    let at = |text: &str, len: u32| {
        let start = src.find(text).unwrap() as u32;
        (start, start + len)
    };
    let basic = code(&lower(src, &Context::default()));
    assert!(basic.contains("tostring(s)"), "{basic}");
    assert!(basic.contains("if __wl_t1 == nil then"), "{basic}");
    let strict = Context {
        fast_forms: HashMap::from([
            (at("s}", 1), STRING_OR_NUMBER | NEVER_FALSE),
            (at("a ??", 1), NEVER_FALSE),
            (at("a?.x", 4), NEVER_FALSE),
        ]),
        ..Context::default()
    };
    let strict = code(&lower(src, &strict));
    assert_eq!(
        strict,
        "local a, b, s = ...\nprint(s .. \"!\", a or b, a ~= nil and a.x or nil)\n"
    );
}

/// An import without its resolution: in the package the module analysis
/// reported why (E1015), so the lowering fails without a second error; a
/// script (no package context) is told that imports need one.
#[test]
fn an_unresolved_import_fails_without_a_second_error() {
    let src = "import foo from \"./missing\"\nprint(foo)\n";
    let mut db = SourceDb::new();
    let file = db.add("t.wowlua", src.to_string());
    let parse = wowlua_syntax::parse(src.as_bytes(), file);
    let module = Context {
        module: Some("t".into()),
        ..Context::default()
    };
    match wowlua_lower::lower_module(&parse, src.as_bytes(), &module) {
        Ok(_) => panic!("an unresolved import lowers"),
        Err(errors) => assert!(errors.is_empty(), "{errors:?}"),
    }
    let errors = wowlua_lower::lower_module(&parse, src.as_bytes(), &Context::default())
        .expect_err("a script's import fails");
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("package context"));
}

/// Directive comments (spec §1.1.1) are the compiler's: a file with a
/// prologue on line 1 (codegen §14.2) leaves them out, so none follows the
/// generated code; other comments stay, and a file without a prologue keeps
/// its directives.
#[test]
fn directives_do_not_follow_the_prologue() {
    let module = Context {
        module: Some("t".into()),
        ..Context::default()
    };
    let src = "--!strict\n-- about t\nexport local x = 1 -- one\n";
    let out = code(&lower(src, &module));
    assert!(!out.contains("--!strict"), "{out}");
    assert!(
        out.starts_with("local __wl = select(2, ...).__wowlua"),
        "{out}"
    );
    assert!(
        out.contains("-- about t\n") && out.contains("-- one"),
        "{out}"
    );
    assert_eq!(out.lines().count(), 3, "{out}");
    let script = "--!strict\nlocal x = 1\nprint(x)\n";
    assert!(code(&lower(script, &Context::default())).starts_with("--!strict\n"));
}
