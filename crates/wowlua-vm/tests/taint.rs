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
const EXCLUDED: &[(&str, &str)] = &[];

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

/// Secure code may call a secret function (decision T195): the function it
/// hides runs, and its results are secret; Blizzard's target frame calls a
/// secret color's `GetRGBA`. Tainted code is refused, naming its taint.
#[test]
fn secure_code_calls_a_secret_function() {
    let mut vm = Vm::wow();
    let check = vm
        .load(
            b"local f = secretwrap(function(a, b) return a + b, 'two' end)
              local color = secretwrap({ r = 1 })
              function color.Get(self) return 2 end
              local x, y, z = f(1, 2)
              local function tail() return f(3, 4) end
              return issecretvalue(x), issecretvalue(y), z, select('#', tail()),
                  issecretvalue(color:Get()), issecretvalue(1)",
            "=check",
        )
        .expect("compiles");
    let r = vm.call(check, &[]).expect("runs");
    let s = |v: Value| match v {
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => n.to_string(),
        other => other.type_name().to_string(),
    };
    assert_eq!(
        r.iter().map(|&v| s(v)).collect::<Vec<_>>(),
        ["true", "true", "nil", "2", "true", "false"]
    );
    let addon = vm
        .load_as(
            b"local f = secretwrap(function() return 1 end) return pcall(f)",
            "@MyAddon/main.lua",
            "MyAddon",
        )
        .expect("compiles");
    let r = vm.call(addon, &[]).expect("runs");
    let Value::Str(msg) = r[1] else {
        panic!("an error message")
    };
    let msg = String::from_utf8_lossy(vm.str_bytes(msg)).into_owned();
    assert!(
        msg.contains("a secret function value") && msg.contains("tainted by 'MyAddon'"),
        "{msg}"
    );
}

thread_local! {
    static DROPPED: std::cell::RefCell<Vec<(String, String)>> = Default::default();
}

fn note_dropped(_vm: &mut Vm, message: String, stack: String) {
    DROPPED.with(|d| d.borrow_mut().push((message, stack)));
}

/// `secureexecuterange` drops its callbacks' errors as the client does (no
/// error handler call, the next entries still run: `forever-camelot.lua`),
/// and tells the host of each, with the stack where it was raised.
#[test]
fn secureexecuterange_tells_the_host_of_dropped_errors() {
    let mut vm = Vm::wow();
    vm.on_dropped_error = Some(note_dropped);
    let f = vm
        .load(
            b"local handled = 0
            seterrorhandler(function() handled = handled + 1 end)
            local n = 0
            secureexecuterange({ 1, 2 }, function(k) n = n + 1 error('entry ' .. k) end)
            return n, handled",
            "@range.lua",
        )
        .expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    assert!(matches!(r[0], Value::Num(n) if n == 2.0), "every entry ran");
    assert!(
        matches!(r[1], Value::Num(n) if n == 0.0),
        "no error reached the handler"
    );
    let dropped = DROPPED.with(|d| d.borrow().clone());
    let messages: Vec<&str> = dropped.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(messages, ["range.lua:4: entry 1", "range.lua:4: entry 2"]);
    assert!(dropped[0].1.contains("range.lua:4"), "{}", dropped[0].1);
}

/// Runs `src` as secure code from a secure start (as the engine starts a
/// script), its results as text.
fn run_secure(vm: &mut Vm, src: &str) -> Vec<String> {
    vm.taint_mut().stack = wowlua_vm::Taint::SECURE;
    let f = vm.load(src.as_bytes(), "=secure").expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    r.iter()
        .map(|&v| match v {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => n.to_string(),
            other => other.type_name().to_string(),
        })
        .collect()
}

fn run_addon(vm: &mut Vm, src: &str) {
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    vm.call(f, &[]).expect("runs");
}

/// A list Blizzard's code keeps, as the group headers' shared
/// `sortingTable`: refilled by `wipe` and `tinsert`, counted with `#`.
const SHARED_LIST: &str = "local list = {}
function Refill(n)
    wipe(list)
    for i = 1, n do tinsert(list, 'unit' .. i) end
end
function Count()
    wipe(list)
    return #list
end
";

/// A slot a secure `wipe` clears keeps the taint of its last writer, and
/// `#` takes the taint of the slot it ends on (issue #275, WoW Forever's
/// `forever-camelot-groupheader2-*.lua`): after an addon's call fills the
/// shared list, secure code that wipes and counts it turns tainted.
#[test]
fn a_wiped_slot_keeps_its_writer_and_the_length_reads_it() {
    let mut vm = Vm::wow();
    run_secure(&mut vm, SHARED_LIST);
    run_secure(&mut vm, "Refill(1)");
    assert_eq!(
        run_secure(&mut vm, "local n = Count() return n, issecure()"),
        ["0", "true"],
        "filled by secure code"
    );
    run_addon(&mut vm, "Refill(1)");
    assert_eq!(
        run_secure(&mut vm, "local n = Count() return n, issecure()"),
        ["0", "false"],
        "filled by the addon's call"
    );
}

/// A secure refill with the same count doesn't clean the list: its
/// `tinsert` counts the wiped list first, turns tainted on the addon's
/// slot, and stores the new element tainted (WoW Forever's `rewrite`).
#[test]
fn a_secure_refill_after_an_addons_stays_tainted() {
    let mut vm = Vm::wow();
    run_secure(&mut vm, SHARED_LIST);
    run_addon(&mut vm, "Refill(1)");
    assert_eq!(
        run_secure(&mut vm, "Refill(1) return issecure()"),
        ["false"],
    );
    assert_eq!(
        run_secure(&mut vm, "local n = Count() return n, issecure()"),
        ["0", "false"],
    );
}

/// `tinsert` stores the value with the caller's taint: removed, the slot
/// it leaves is the addon's, and the length reads it; the length of a full
/// list reads no nil slot, and its last element doesn't taint it.
#[test]
fn tinsert_stores_with_the_callers_taint() {
    let mut vm = Vm::wow();
    run_secure(&mut vm, "Shared = {} Keyed = {} Removed = {}");
    run_addon(
        &mut vm,
        "tinsert(Shared, 'x') tinsert(Keyed, 'x') Keyed.k = 'y' \
         tinsert(Removed, 'x') tinsert(Removed, 'y') Removed[2] = nil",
    );
    assert_eq!(
        run_secure(
            &mut vm,
            "local t = {} t[1] = 'z' local a, by = issecurevariable(Shared, 1) \
             return #t, issecure(), tostring(rawget(Shared, 1)), tostring(by)"
        ),
        ["1", "true", "x", "nil"],
        "issecurevariable reads the number key as the string \"1\""
    );
    assert_eq!(
        run_secure(&mut vm, "local _ = #Shared return issecure()"),
        ["true"],
        "a full list: no nil slot read"
    );
    assert_eq!(
        run_secure(&mut vm, "local _ = #Keyed return issecure()"),
        ["true"],
        "the slot past the last element, never written, ends the search"
    );
    assert_eq!(
        run_secure(&mut vm, "local n = #Removed return n, issecure()"),
        ["1", "false"],
        "the addon's cleared slot ends the search"
    );
}
