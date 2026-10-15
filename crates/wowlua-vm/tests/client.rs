//! The client's behaviour as a fixture (spec emulator §17.8): StdTest's
//! `/stdtest probe` runs the conformance cases of
//! `tests/ingame/src/conformance.lua` in the game and saves what each
//! returned or raised; `tests/client/<client>.lua` holds those results, and
//! the VM, in that client's environment, must give the same.

use std::collections::BTreeMap;
use std::path::PathBuf;

use wowlua_vm::{Value, Vm};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Loads the cases file as the client does, with a stand-in for wowlua's
/// module runtime, and runs it: each case's result and each function's kind.
/// `/stdtest probe` runs inside two `xpcall`s (the slash command's and the
/// command's error handling), whose nested C calls count toward the limits
/// the cases measure; so does this.
const DRIVER: &str = "local chunk = ...
local module
local ns = { __wowlua = { define = function(_, m) module = m end } }
chunk('StdTest', ns)
local results, kinds
local function keep(e) return e end
xpcall(function()
    xpcall(function() results, kinds = module.run(), module.kinds() end, keep)
end, keep)
return results, kinds";

fn strings(vm: &Vm, t: Value) -> BTreeMap<String, String> {
    let Value::Table(t) = t else {
        panic!("a table expected")
    };
    vm.heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(k), Value::Str(v)) => Some((
                String::from_utf8_lossy(vm.str_bytes(k)).into_owned(),
                String::from_utf8_lossy(vm.str_bytes(v)).into_owned(),
            )),
            _ => None,
        })
        .collect()
}

/// Our results and kinds.
fn ours() -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let src = std::fs::read(repo().join("tests/ingame/src/conformance.lua")).expect("the cases");
    let mut vm = Vm::wow();
    let chunk = vm
        .load(&src, "@conformance.lua")
        .expect("the cases compile");
    let driver = vm.load(DRIVER.as_bytes(), "=driver").expect("compiles");
    let r = vm.call(driver, &[chunk]).unwrap_or_else(|e| {
        let msg = match e.value {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => other.type_name().to_string(),
        };
        panic!("the cases fail to run: {msg}")
    });
    (strings(&vm, r[0]), strings(&vm, r[1]))
}

/// A fixture: `return { client = …, interface = …, results = {…}, kinds = {…} }`.
fn fixture(name: &str) -> Option<(BTreeMap<String, String>, BTreeMap<String, String>)> {
    let path = repo().join(format!("crates/wowlua-vm/tests/client/{name}.lua"));
    let src = std::fs::read(path).ok()?;
    let mut vm = Vm::new();
    let f = vm.load(&src, "=fixture").expect("the fixture compiles");
    let r = vm.call(f, &[]).expect("the fixture runs");
    let Value::Table(t) = r[0] else {
        panic!("the fixture returns a table")
    };
    let get = |vm: &mut Vm, key: &str| {
        let k = vm.str(key);
        vm.heap.table(t).get(k)
    };
    let results = get(&mut vm, "results");
    let kinds = get(&mut vm, "kinds");
    Some((strings(&vm, results), strings(&vm, kinds)))
}

/// Cases whose behaviour the VM alone does not have, with why: they are
/// listed, not failed (`wowlua-engine`'s `tests/client.rs` runs every case
/// with the engine and Blizzard's UI code).
const PENDING: &[(&str, &str)] = &[
    ("CreateFrame(", "frames are the engine's (S3)"),
    (
        "local m = getmetatable('')",
        "string.rtgsub is Blizzard's Lua (S3)",
    ),
    (
        "assertf",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "ipairs_reverse",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "getglobal",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "setglobal",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "nop(",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "tostringall",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "getprinthandler",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "rtgsub",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "coroutine.create(print)",
        "the client's print is Blizzard's Lua (S3)",
    ),
    (
        "includes, mapvalues",
        "Blizzard's Lua and engine functions (S3)",
    ),
    (
        "issecurevariable('StdTestConformanceGlobal')",
        "written by setglobal, Blizzard's Lua (S3)",
    ),
    (
        "issecurevariable('StdTestState')",
        "saved variables load as the addon's (S3)",
    ),
];

#[test]
fn forever_camelot() {
    let Some((results, _kinds)) = fixture("forever-camelot") else {
        return;
    };
    let (ours, _) = ours();
    let mut differences = Vec::new();
    let mut pending = 0;
    for (source, theirs) in &results {
        if PENDING.iter().any(|(p, _)| source.contains(p)) {
            pending += 1;
            continue;
        }
        match ours.get(source) {
            Some(mine) if mine == theirs => {}
            mine => differences.push(format!(
                "{source}\n    client: {theirs}\n    ours:   {}",
                mine.map_or("<not run>", String::as_str)
            )),
        }
    }
    assert!(
        differences.is_empty(),
        "{} of {} cases differ from the client ({pending} pending):\n{}",
        differences.len(),
        results.len(),
        differences.join("\n")
    );
    println!(
        "{} cases match the client; {pending} pending",
        results.len() - pending
    );
}

/// Prints our results (`cargo test -p wowlua-vm --test client -- --ignored
/// --nocapture`).
#[test]
#[ignore]
fn print_ours() {
    let (results, kinds) = ours();
    for (source, result) in &results {
        println!("{source}\n    {result}");
    }
    println!("{} cases, {} functions", results.len(), kinds.len());
}

/// Where the client crashes (`/stdtest crashlimit`, WoW Forever 1.60.1):
/// `pcall` returning `#args + 100` or more values, `xpcall` one fewer (its
/// handler takes a slot), `pcallwithenv` three fewer (its status, the
/// function and its environment). The VM raises an error
/// naming the call instead (decision T194).
#[test]
fn pcall_crash_limit() {
    let src = "local t, u = {}, unpack for i = 1, 200 do t[i] = i end
local function f(n) return unpack(t, 1, n) end
local out = {}
out[1] = select('#', pcall(unpack, t, 1, 103))
out[2] = select(2, pcall(function() return pcall(unpack, t, 1, 104) end))
out[3] = select('#', pcall(f, 101))
out[4] = select(2, pcall(function() return pcall(f, 102) end))
out[5] = select('#', xpcall(f, print, 101))
out[6] = select(2, pcall(function() return xpcall(f, print, 102) end))
local function g(n) return u(t, 1, n) end
out[7] = select('#', pcallwithenv(g, {}, 99))
out[8] = select(2, pcall(function() return pcallwithenv(g, {}, 100) end))
out[9] = select('#', pcallwithenv(g, {}, 101, 0, 0))
out[10] = select(2, pcall(function() return pcallwithenv(g, {}, 102, 0, 0) end))
return table.concat(out, '|')";
    let mut vm = Vm::wow();
    let f = vm.load(src.as_bytes(), "=crash").expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let Value::Str(s) = r[0] else {
        panic!("a string expected")
    };
    let assertion =
        "results crash the game (its Lua assertion \"L->top < L->ci->top\", lapi.c:577)";
    assert_eq!(
        String::from_utf8_lossy(vm.str_bytes(s)),
        format!(
            "104|crash:5: pcall: 104 {assertion}|102|crash:7: pcall: 102 {assertion}|102|crash:9: xpcall: 102 {assertion}|100|crash:12: pcallwithenv: 100 {assertion}|102|crash:14: pcallwithenv: 102 {assertion}"
        )
    );
}

/// The client's nested C call limit (`/stdtest crashlimit` 14 to 16, WoW
/// Forever 1.60.1), from the command's context: the slash command's two
/// `xpcall`s and the experiment's `pcall`, then a coroutine per try, which
/// continues the caller's count.
#[test]
fn c_call_limit() {
    let src = "local function ascend(nest)
    local last, d = 0, 1
    while d <= 300 do
        local ok, result = coroutine.wrap(function() return pcall(nest, d) end)()
        if not ok or result ~= 'ok' then return last .. ' ' .. tostring(result) end
        last, d = d, d + 1
    end
    return last
end
local function viaPcall(d)
    if d == 0 then return 'ok' end
    local ok, r = pcall(viaPcall, d - 1)
    return ok and r or tostring(r)
end
local proxy = setmetatable({}, { __index = function(t, k) if k == 0 then return 'ok' end return t[k - 1] end })
local function viaResume(d)
    if d == 0 then return 'ok' end
    local ok, r = coroutine.resume(coroutine.create(viaResume), d - 1)
    return ok and r or tostring(r)
end
local out
local function keep(e) return e end
xpcall(function() xpcall(function()
    local _, r = pcall(function()
        return ascend(viaPcall) .. '|' .. ascend(function(d) return proxy[d] end) .. '|' .. ascend(viaResume)
    end)
    out = r
end, keep) end, keep)
return out";
    // Each nested C call recurses in Rust: 200 of them need more than a
    // test thread's stack (as in `xtask corpus lua51`).
    let out = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            let mut vm = Vm::wow();
            let f = vm.load(src.as_bytes(), "=depth").expect("compiles");
            let r = vm.call(f, &[]).expect("runs");
            let Value::Str(s) = r[0] else {
                panic!("a string expected")
            };
            String::from_utf8_lossy(vm.str_bytes(s)).into_owned()
        })
        .expect("a thread")
        .join()
        .expect("no panic");
    assert_eq!(
        out,
        "193 C stack overflow|192 depth:15: C stack overflow|194 C stack overflow"
    );
}
