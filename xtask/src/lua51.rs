//! `cargo xtask corpus lua51 [dir]`: the official Lua 5.1 test suite
//! (`lua51-tests` in the corpus) on wowlua's VM (spec emulator §17.8).
//!
//! Each file runs in a fresh VM with the WoW environment plus the test-only
//! shims in `PRELUDE`, for what the suite needs only as plumbing: timing,
//! output, temporary files kept in memory, loading its own helper files.
//! What WoW's Lua does not have is excluded explicitly, with the reason:
//! whole files in `EXCLUDED_FILES`, line ranges in `PATCHES` (blanked, so
//! line numbers in messages stay the same). Everything else must pass.
//! `WOWLUA_LUA51_FILE=<name>` runs one file and shows its output.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::time::Instant;
use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

/// Files that test what WoW's Lua does not have at all.
const EXCLUDED_FILES: &[(&str, &str)] = &[
    (
        "all.lua",
        "the driver: loadfile, package.path, debug.sethook",
    ),
    ("main.lua", "the standalone interpreter"),
    ("files.lua", "the io and os libraries"),
    ("db.lua", "the debug library"),
];

/// Line ranges (inclusive) that test what WoW's Lua does not have: blanked,
/// and the first line replaced by the given text.
const PATCHES: &[(&str, u32, u32, &str, &str)] = &[
    ("attrib.lua", 1, 235, "", "require, module and package"),
    (
        "big.lua",
        1,
        12,
        "",
        "a 32-bit size_t overflow (64-bit Lua fails it too)",
    ),
    ("calls.lua", 241, 241, "", "debug.getinfo"),
    ("calls.lua", 243, 246, "", "string.dump"),
    (
        "calls.lua",
        271,
        285,
        "",
        "string.dump and debug.setupvalue",
    ),
    ("closure.lua", 276, 277, "", "debug.getinfo"),
    ("closure.lua", 416, 419, "", "debug.setfenv on a coroutine"),
    ("constructs.lua", 189, 189, "", "debug.getinfo"),
    ("errors.lua", 111, 111, "", "io.stdin"),
    (
        "errors.lua",
        142,
        142,
        "function y () C=C+1; y() end",
        "debug.getinfo",
    ),
    (
        "errors.lua",
        149,
        167,
        "",
        "debug.getinfo and debug.traceback",
    ),
    (
        "events.lua",
        330,
        355,
        "",
        "debug.setmetatable on basic types",
    ),
    (
        "literals.lua",
        112,
        112,
        r#"return tonumber(string.match(select(2, pcall(function () return nil + 1 end)), ":(%d+):"))"#,
        "debug.getinfo (the line is read from an error's position instead)",
    ),
    ("math.lua", 124, 142, "", "io.tmpfile"),
    (
        "pm.lua",
        226,
        226,
        "",
        "string.gfind is an error in the client",
    ),
    (
        "vararg.lua",
        5,
        10,
        "",
        "the implicit `arg` (LUA_COMPAT_VARARG)",
    ),
    (
        "vararg.lua",
        20,
        20,
        "",
        "the implicit `arg` (LUA_COMPAT_VARARG)",
    ),
    (
        "vararg.lua",
        24,
        26,
        "",
        "the implicit `arg` (LUA_COMPAT_VARARG)",
    ),
    (
        "vararg.lua",
        36,
        40,
        "",
        "the implicit `arg` (LUA_COMPAT_VARARG)",
    ),
    (
        "vararg.lua",
        44,
        46,
        "",
        "the implicit `arg` (LUA_COMPAT_VARARG)",
    ),
];

/// How `all.lua` runs each file, with the results it checks.
const DRIVERS: &[(&str, &str)] = &[
    ("attrib.lua", "assert(dofile('attrib.lua') == 27)"),
    (
        "big.lua",
        "local f = coroutine.wrap(assert(loadfile('big.lua'))) \
         assert(f() == 'b') assert(f() == 'a')",
    ),
    ("calls.lua", "assert(dofile('calls.lua') == deep and deep)"),
    ("events.lua", "assert(dofile('events.lua') == 12)"),
    ("gc.lua", "local f = assert(loadfile('gc.lua')) f()"),
    ("locals.lua", "assert(dofile('locals.lua') == 5)"),
    (
        "verybig.lua",
        "assert(dofile('verybig.lua') == 10) collectgarbage()",
    ),
];

/// Test-only plumbing. `math.mod` is stock Lua 5.1's LUA_COMPAT_MOD alias,
/// which the suite uses as a helper; the client has only `math.fmod`.
const PRELUDE: &str = r##"
local suite_source, clock, time = ...
math.mod = math.fmod
local files, output, ntmp = {}, nil, 0
os = {
  clock = clock,
  time = time,
  setlocale = function (l) if l == nil or l == "" or l == "C" then return "C" end end,
  tmpname = function () ntmp = ntmp + 1; return "tmp" .. ntmp end,
  remove = function (name) files[name] = nil; return true end,
}
io = {
  output = function (name) output = name; files[name] = {} end,
  write = function (...)
    if output then
      local b = files[output]
      for i = 1, select("#", ...) do b[#b + 1] = tostring((select(i, ...))) end
    end
  end,
  close = function () output = nil end,
}
function loadfile (name)
  local src = files[name] and table.concat(files[name]) or suite_source(name)
  return loadstring(src, "@" .. name)
end
function dofile (name)
  return assert(loadfile(name))()
end
local loaded = {}
function require (name)
  if not loaded[name] then loaded[name] = dofile(name .. ".lua") or true end
  return loaded[name]
end
"##;

thread_local! {
    static STARTED: Instant = Instant::now();
    static SUITE: std::cell::RefCell<PathBuf> = std::cell::RefCell::default();
}

fn clock(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(
        STARTED.with(|s| s.elapsed().as_secs_f64()),
    )])
}

fn time(_vm: &mut Vm, _c: NativeCall) -> Ret {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Ok(vec![Value::Num(now as f64)])
}

/// A suite file's source, patched.
fn suite_source(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Str(s) = c.arg(1) else {
        return Err(vm.arg_error(1, "string expected"));
    };
    let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
    let path = SUITE.with(|d| d.borrow().join(&name));
    match std::fs::read(&path) {
        Ok(text) => Ok(vec![vm.bytes(&source(&name, &text))]),
        Err(e) => Err(vm.error(&format!("cannot read {name}: {e}"))),
    }
}

/// The file as `luaL_loadfile` reads it, patched.
fn source(name: &str, text: &[u8]) -> Vec<u8> {
    let mut lines: Vec<&[u8]> = text.split(|&b| b == b'\n').collect();
    // A first line starting with `#` is skipped; its newline stays.
    if lines.first().is_some_and(|l| l.starts_with(b"#")) {
        lines[0] = b"";
    }
    for &(file, from, to, replacement, _) in PATCHES {
        if file == name {
            for l in &mut lines[from as usize - 1..to as usize] {
                *l = b"";
            }
            lines[from as usize - 1] = replacement.as_bytes();
        }
    }
    lines.join(&b'\n')
}

fn run_file(suite: &Path, name: &str, show_output: bool) -> Result<(), String> {
    SUITE.with(|d| *d.borrow_mut() = suite.to_path_buf());
    let mut vm = Vm::new();
    vm.set_output(move |b| {
        if show_output {
            print!("{}", String::from_utf8_lossy(b));
        }
    });
    let message = |vm: &Vm, e: wowlua_vm::LuaError| match e.value {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        other => format!("({})", other.type_name()),
    };
    let prelude = vm.load(PRELUDE.as_bytes(), "=prelude")?;
    let args = [
        vm.native("suite_source", suite_source),
        vm.native("clock", clock),
        vm.native("time", time),
    ];
    vm.call(prelude, &args).map_err(|e| message(&vm, e))?;
    let driver = DRIVERS
        .iter()
        .find(|d| d.0 == name)
        .map_or_else(|| format!("dofile('{name}')"), |d| d.1.to_string());
    let driver = vm.load(driver.as_bytes(), "=driver")?;
    vm.call(driver, &[])
        .map(|_| ())
        .map_err(|e| message(&vm, e))
}

pub fn run(dir: &Path) -> Result<()> {
    let suite = dir.join("lua51-tests/lua5.1-tests");
    let mut names: Vec<String> = std::fs::read_dir(&suite)?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".lua"))
        .collect();
    names.sort();
    // One file, with its output shown.
    let only = std::env::var("WOWLUA_LUA51_FILE").ok();
    if let Some(only) = &only {
        names.retain(|n| n == only);
    }
    let mut failed = 0;
    for name in &names {
        if let Some((_, why)) = EXCLUDED_FILES.iter().find(|(f, _)| f == name) {
            println!("{name:16} excluded: {why}");
            continue;
        }
        let started = Instant::now();
        // A thread for the stack: some tests recurse deeply.
        let (n, s) = (name.clone(), suite.clone());
        let show = only.is_some();
        let r = std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(move || run_file(&s, &n, show))?
            .join()
            .unwrap_or_else(|_| Err("panicked".into()));
        let patched: u32 = PATCHES
            .iter()
            .filter(|p| p.0 == name)
            .map(|p| p.2 - p.1 + 1)
            .sum();
        let note = if patched > 0 {
            format!(", {patched} lines excluded")
        } else {
            String::new()
        };
        match r {
            Ok(()) => println!("{name:16} ok ({:.1?}{note})", started.elapsed()),
            Err(e) => {
                failed += 1;
                println!("{name:16} FAILED: {e}");
            }
        }
    }
    if failed > 0 {
        bail!("{failed} files failed");
    }
    Ok(())
}
