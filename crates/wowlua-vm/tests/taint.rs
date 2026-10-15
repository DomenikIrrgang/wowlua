//! Taint conformance (spec emulator §17.4): elune's script cases
//! (`tests/elune/luatest_scriptcases.lua`), which pass in the WoW client
//! when run securely, on the VM with the client's environment.

use std::path::PathBuf;

use wowlua_vm::{Value, Vm};

/// `case(name, f)` as elune's runner defines it: `f` runs through
/// `securecall`, and what reaches the error handler fails the case.
const PRELUDE: &str = "local results = {}
function case(name, func)
    local errors = {}
    local old = geterrorhandler()
    seterrorhandler(function(err) errors[#errors + 1] = tostring(err) end)
    securecall(func)
    seterrorhandler(old)
    results[#results + 1] = name
    results[name] = table.concat(errors, '; ')
end
return results";

/// Cases for what the client does not have.
const EXCLUDED: &[(&str, &str)] = &[(
    "loadstring_untainted: does not taint caller",
    "elune's own function; the client has no loadstring_untainted",
)];

#[test]
fn elune_script_cases() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/elune/luatest_scriptcases.lua");
    let src = std::fs::read(path).expect("the cases");
    let mut vm = Vm::wow();
    let prelude = vm.load(PRELUDE.as_bytes(), "=prelude").expect("compiles");
    let results = vm.call(prelude, &[]).expect("runs")[0];
    let cases = vm
        .load(&src, "@luatest_scriptcases.lua")
        .expect("the cases compile");
    if let Err(e) = vm.call(cases, &[]) {
        let msg = match e.value {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => other.type_name().to_string(),
        };
        panic!("the cases fail to run: {msg}");
    }
    let Value::Table(t) = results else {
        panic!("results")
    };
    let n = vm.heap.table(t).border();
    let mut failures = Vec::new();
    for i in 1..=n {
        let Value::Str(name) = vm.heap.table(t).get_int(i) else {
            continue;
        };
        let errors = vm.heap.table(t).get_str(name);
        let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
        let errors = match errors {
            Value::Str(e) => String::from_utf8_lossy(vm.str_bytes(e)).into_owned(),
            _ => String::new(),
        };
        if !errors.is_empty() && !EXCLUDED.iter().any(|(c, _)| *c == name) {
            failures.push(format!("{name}: {errors}"));
        }
    }
    assert!(n > 100, "only {n} cases ran");
    assert!(
        failures.is_empty(),
        "{} of {n} cases fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Code loaded as an addon's taints what it writes; secure code reading it
/// becomes tainted, and `issecurevariable` names the addon.
#[test]
fn addon_code_taints_its_writes() {
    let mut vm = Vm::wow();
    let addon = vm
        .load_as(
            b"AddonGlobal = 1 function AddonFunc() end",
            "@MyAddon/main.lua",
            "MyAddon",
        )
        .expect("compiles");
    vm.call(addon, &[]).expect("runs");
    // Taint persists after the addon's code ran; the engine starts each
    // script it runs secure.
    vm.taint_mut().stack = wowlua_vm::Taint::SECURE;
    let check = vm
        .load(
            b"local ok, who = issecurevariable('AddonGlobal')
              local before = issecure()
              local _ = AddonGlobal
              return ok, who, before, issecure(), issecurevariable('print')",
            "=check",
        )
        .expect("compiles");
    let r = vm.call(check, &[]).expect("runs");
    let s = |v: Value| match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Bool(b) => b.to_string(),
        other => other.type_name().to_string(),
    };
    assert_eq!(
        r.iter().map(|&v| s(v)).collect::<Vec<_>>(),
        ["false", "MyAddon", "true", "false", "true", "nil"]
    );
}
