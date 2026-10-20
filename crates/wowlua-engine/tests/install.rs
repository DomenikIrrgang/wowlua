//! The engine's API from the documentation.

use wowlua_vm::{Value, Vm};

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "=test").expect("compiles");
    let r = vm.call(f, &[]).unwrap_or_else(|e| match e.value {
        Value::Str(s) => panic!("{}", String::from_utf8_lossy(vm.str_bytes(s))),
        _ => panic!("an error"),
    });
    r.into_iter()
        .map(|v| match v {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => format!("{other:?}"),
        })
        .collect()
}

#[test]
fn documented_functions_exist_and_are_not_mocked() {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    // Addon code: a function without behaviour fails.
    let f = vm
        .load_as(
            b"return type(IsCollisionEnabled), select(2, pcall(IsCollisionEnabled)), type(GetTime), type(secretwrap)",
            "@MyAddon/main.lua",
            "MyAddon",
        )
        .expect("compiles");
    let r: Vec<String> = vm
        .call(f, &[])
        .expect("runs")
        .into_iter()
        .map(|v| match v {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        r,
        [
            "function",
            "IsCollisionEnabled is not mocked",
            "function",
            "function"
        ]
    );
    // Secure code (a fresh execution): its documented results' empty values.
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let r = run(&mut vm, "return IsCollisionEnabled()");
    assert_eq!(r, ["Bool(false)"]);
}

#[test]
fn enums_and_constants() {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let r = run(
        &mut vm,
        "return tostring(Enum.AddOnRestrictionType.Combat), tostring(Enum.AddOnRestrictionType.Map), tostring(Enum.TableSecurityOption.SecretWrapContents)",
    );
    assert_eq!(r, ["0", "4", "2"]);
}

#[test]
fn every_flavor_installs() {
    for flavor in [
        "standard", "camelot", "mists", "cata", "wrath", "tbc", "vanilla",
    ] {
        let mut vm = Vm::wow();
        wowlua_engine::install(&mut vm, flavor).expect("installs");
    }
}
