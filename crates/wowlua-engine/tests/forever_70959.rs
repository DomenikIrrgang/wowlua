//! What WoW Forever 1.60.1 70959 added that Blizzard's UI calls (issue
//! #285, decision T645): `C_TradeSkillUI.HasNoReagentCost` (the
//! professions window's Create All and tracking) and `C_Mail.IsCODEnabled`
//! (the send frame's COD button), with their defaults and the tests'
//! setters; camelot's alone.

use wowlua_vm::{Value, Vm};

fn vm(flavor: &str) -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, flavor).expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn text(vm: &Vm, v: Value) -> String {
    match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{n}"),
        Value::Bool(b) => format!("{b}"),
        Value::Nil => "nil".into(),
        other => format!("{other:?}"),
    }
}

/// Runs `src` as an addon's code (`secure`: as Blizzard's).
fn run(vm: &mut Vm, src: &str, secure: bool) -> Result<Vec<String>, String> {
    let f = if secure {
        vm.load(src.as_bytes(), "=secure").expect("compiles")
    } else {
        vm.load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
            .expect("compiles")
    };
    match vm.call_secure(f, &[]) {
        Ok(r) => Ok(r.into_iter().map(|v| text(vm, v)).collect()),
        Err(e) => Err(text(vm, e.value)),
    }
}

/// A recipe has reagents unless a test says it costs none; the setting is
/// per recipe and taken back with false.
#[test]
fn a_recipe_costs_reagents_unless_set() {
    let mut vm = vm("camelot");
    let r = run(
        &mut vm,
        "local has = C_TradeSkillUI.HasNoReagentCost
local before = has(2738)
mock.setNoReagentCost(2738)
local set, other = has(2738), has(2739)
mock.setNoReagentCost(2739, true)
mock.setNoReagentCost(2738, false)
return type(before), before, set, other, has(2738), has(2739)",
        false,
    )
    .unwrap();
    assert_eq!(r, ["boolean", "false", "true", "false", "false", "true"]);
    // Blizzard's own code gets the same answer.
    let r = run(
        &mut vm,
        "return C_TradeSkillUI.HasNoReagentCost(2739), C_TradeSkillUI.HasNoReagentCost(3326)",
        true,
    )
    .unwrap();
    assert_eq!(r, ["true", "false"]);
    let err = run(&mut vm, "return C_TradeSkillUI.HasNoReagentCost()", false).unwrap_err();
    assert!(err.contains("bad argument #1"), "{err}");
}

/// Letters can be sent cash on delivery unless a test turns it off.
#[test]
fn cod_is_enabled_unless_turned_off() {
    let mut vm = vm("camelot");
    let r = run(
        &mut vm,
        "local before = C_Mail.IsCODEnabled()
mock.setCODEnabled(false)
local off = C_Mail.IsCODEnabled()
mock.setCODEnabled(true)
return type(before), before, off, C_Mail.IsCODEnabled()",
        false,
    )
    .unwrap();
    assert_eq!(r, ["boolean", "true", "false", "true"]);
    let r = run(&mut vm, "return C_Mail.IsCODEnabled()", true).unwrap();
    assert_eq!(r, ["true"]);
    let err = run(&mut vm, "mock.setCODEnabled(nil)", false).unwrap_err();
    assert!(err.contains("setCODEnabled"), "{err}");
}

/// The other flavors' clients have neither function.
#[test]
fn only_camelot_has_them() {
    for flavor in ["standard", "mists", "tbc", "vanilla"] {
        let mut vm = vm(flavor);
        let r = run(
            &mut vm,
            "return C_TradeSkillUI and C_TradeSkillUI.HasNoReagentCost, C_Mail and C_Mail.IsCODEnabled",
            false,
        )
        .unwrap();
        assert!(
            r.iter().all(|v| v == "nil" || v == "false"),
            "{flavor}: {r:?}"
        );
    }
}
