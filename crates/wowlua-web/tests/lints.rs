//! The playground reports what `wowlua check` and `wowlua build` report for
//! the same package (spec M14 S2): the manifest's `check.lints` apply, and
//! W2063 (an implicit `any`) is off until they turn it on. One test: the
//! files' host is the program's.

use serde_json::Value as Json;
use wowlua_web::Playground;

const MAIN: &str = "local function show(value)\n    print(value)\nend\nshow(1)\nlocal unused = 2\n";

fn manifest(lints: &str) -> String {
    format!(
        r#"{{
    "package": {{ "name": "playground", "version": "0.1.0", "wowlua": "^0.12" }},
    "flavors": {{ "camelot": {{}} }},
    "check": {{ "lints": {{ {lints} }} }}
}}
"#
    )
}

/// A check's or build's diagnostics as `severity code`.
fn found(diagnostics: &Json) -> Vec<String> {
    diagnostics
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            format!(
                "{} {}",
                d["severity"].as_str().unwrap(),
                d["code"].as_str().unwrap()
            )
        })
        .collect()
}

fn run(p: &mut Playground) -> Json {
    p.run(wowlua_web::run::Options {
        flavor: "camelot".into(),
        blizzard_ui: false,
        profile: None,
        debug: None,
        installation: None,
    })
}

#[test]
fn the_manifest_s_lints_apply_as_in_the_cli() {
    let mut p = Playground::new();
    p.set_time(1_800_000_000_000.0, 0.0);
    p.write_file("src/main.wowlua", MAIN);

    // by default: no W2063
    p.write_file("wowlua.json", &manifest(""));
    assert_eq!(found(&p.check()), ["warning W1003"]);
    assert_eq!(found(&p.build("dev")["diagnostics"]), ["warning W1003"]);
    assert_eq!(found(&run(&mut p)["diagnostics"]), ["warning W1003"]);
    p.stop();

    // turned on; another allowed
    p.write_file(
        "wowlua.json",
        &manifest(r#""W2063": "warn", "W1003": "allow""#),
    );
    assert_eq!(found(&p.check()), ["warning W2063"]);
    assert_eq!(found(&p.build("dev")["diagnostics"]), ["warning W2063"]);

    // denied: an error, and nothing is built or run
    p.write_file("wowlua.json", &manifest(r#""W1003": "deny""#));
    assert_eq!(found(&p.check()), ["error W1003"]);
    let built = p.build("dev");
    assert_eq!(built["ok"], false, "{built}");
    let ran = run(&mut p);
    assert_eq!(ran["ok"], false, "{ran}");
    assert!(p.game().is_none());
}
