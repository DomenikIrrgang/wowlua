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
    let r = run(
        &mut vm,
        "return type(C_Timer.After), select(2, pcall(C_Timer.After, 1, print)), type(GetTime), type(secretwrap)",
    );
    assert_eq!(
        r,
        [
            "function",
            "C_Timer.After is not mocked",
            "function",
            "function"
        ]
    );
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
