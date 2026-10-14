//! The monitor (spec §7.9) routes `SetScript` and `HookScript` method calls
//! with two arguments in parentheses; every other method call stays as
//! written.

use wowlua_lower::{Context, Tools};
use wowlua_luair::{EmitOptions, emit};
use wowlua_span::SourceDb;

fn monitored(src: &str) -> String {
    let mut db = SourceDb::new();
    let file = db.add("t.lua", src.to_string());
    let parse = wowlua_syntax::parse(src.as_bytes(), file);
    assert!(!parse.has_errors(), "{src}\n{:?}", parse.diagnostics);
    let ctx = Context {
        tools: Tools {
            monitor: true,
            ..Tools::default()
        },
        ..Context::default()
    };
    let chunk = wowlua_lower::lower_with(&parse, src.as_bytes(), &ctx).expect("lowers");
    String::from_utf8(emit(&chunk, &EmitOptions::default()).text).unwrap()
}

#[test]
fn only_two_argument_script_calls_are_routed() {
    let code = monitored(
        r#"local f = CreateFrame("Frame")
f:SetScript("OnEvent", print)
f:HookScript("OnShow", print)
f:SetScript("OnHide")
f:SetScript("OnUpdate", print, 1)
f:SetScript{"OnClick"}
f:SetScript"OnEnter"
f:HookScript()
f:RegisterEvent("PLAYER_LOGIN", print)
"#,
    );
    let lines: Vec<&str> = code.lines().collect();
    assert_eq!(
        lines[1..9],
        [
            "__wl.mon.setScript(f, \"OnEvent\", print, \":2\")",
            "__wl.mon.hookScript(f, \"OnShow\", print, \":3\")",
            "f:SetScript(\"OnHide\")",
            "f:SetScript(\"OnUpdate\", print, 1)",
            "f:SetScript{\"OnClick\"}",
            "f:SetScript\"OnEnter\"",
            "f:HookScript()",
            "f:RegisterEvent(\"PLAYER_LOGIN\", print)",
        ],
        "{code}"
    );
}
