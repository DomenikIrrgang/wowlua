//! WoW's changes to Lua 5.1's environment (spec emulator §17.3): the
//! client's rules ([`Rules`]), the libraries it removes, and what it adds.
//! Native where the client's are native (elune, `liblua`, is the reference
//! for those), Lua code where the client's are Lua code; each behaviour is
//! checked against the client's recorded conformance results
//! (`tests/client.rs`).

use crate::error::LuaError;
use crate::function::NativeCall;
use crate::heap::TableRef;
use crate::stdlib::{Ret, register};
use crate::value::Value;
use crate::vm::{Rules, Vm};

/// The client's aliases and wrappers, as elune defines them (`lcompatlib.c`):
/// Lua code, so they behave as Lua functions do (the trigonometry works in
/// degrees).
const COMPAT: &str = "local table = table
foreach = table.foreach
foreachi = table.foreachi
getn = table.getn
tinsert = table.insert
tremove = table.remove
sort = table.sort
wipe = table.wipe
local math = math
abs = math.abs
acos = function (x) return math.deg(math.acos(x)) end
asin = function (x) return math.deg(math.asin(x)) end
atan = function (x) return math.deg(math.atan(x)) end
atan2 = function (x,y) return math.deg(math.atan2(x,y)) end
ceil = math.ceil
cos = function (x) return math.cos(math.rad(x)) end
deg = math.deg
exp = math.exp
floor = math.floor
frexp = math.frexp
ldexp = math.ldexp
log = math.log
log10 = math.log10
max = math.max
min = math.min
mod = math.fmod
PI = math.pi
rad = math.rad
random = math.random
sin = function (x) return math.sin(math.rad(x)) end
sqrt = math.sqrt
tan = function (x) return math.tan(math.rad(x)) end
local string = string
strbyte = string.byte
strchar = string.char
strfind = string.find
format = string.format
gmatch = string.gmatch
gsub = string.gsub
strlen = string.len
strlower = string.lower
strmatch = string.match
strrep = string.rep
strrev = string.reverse
strsub = string.sub
strupper = string.upper
string.trim = strtrim
";

impl Vm {
    /// A VM with the WoW client's Lua environment.
    pub fn wow() -> Vm {
        let mut vm = Vm::new();
        open(&mut vm);
        vm
    }
}

pub fn open(vm: &mut Vm) {
    vm.rules = Rules {
        fpe_strict: true,
        fpe_coerce: true,
        int_overflow: true,
        client_format: true,
        xpcall_args: true,
        client_errors: true,
        native_coroutines: true,
        utf8_case: true,
    };
    // Reads taint the execution, writes take its taint.
    vm.taint_mut().mode = crate::taint::READ | crate::taint::WRITE;
    crate::stdlib::secure::open(vm);
    // What the client does not have.
    vm.set_global("load", Value::Nil);
    let math = global_table(vm, "math");
    set(vm, math, "randomseed", Value::Nil);
    let string = global_table(vm, "string");
    let gfind = vm.native("gfind", gfind);
    set(vm, string, "gfind", gfind);

    for (name, f) in [
        ("strsplit", strsplit as crate::NativeFn),
        ("strsplittable", strsplittable),
        ("strjoin", strjoin),
        ("strtrim", strtrim),
        ("strconcat", strconcat),
        ("strlenutf8", strlenutf8),
        ("strcmputf8i", strcmputf8i),
        ("fastrandom", fastrandom),
        ("date", date),
        ("time", time),
        ("difftime", difftime),
        ("debugstack", debugstack),
        ("debuglocals", debuglocals),
        ("pcallwithenv", pcallwithenv),
        ("geterrorhandler", geterrorhandler),
        ("seterrorhandler", seterrorhandler),
        ("debugprofilestart", debugprofilestart),
        ("debugprofilestop", debugprofilestop),
    ] {
        let v = vm.native(name, f);
        vm.set_global(name, v);
    }
    let table = global_table(vm, "table");
    for (name, f) in [
        ("wipe", wipe as crate::NativeFn),
        ("removemulti", removemulti),
        ("count", count),
        ("getcountinfo", getcountinfo),
        ("create", create),
        ("contains", contains),
        ("indexof", indexof),
        ("isempty", isempty),
        ("keys", keys),
        ("values", values),
        ("removevalue", removevalue),
        ("removeunordered", removeunordered),
        ("freeze", freeze),
        ("isfrozen", isfrozen),
    ] {
        let v = vm.native(name, f);
        set(vm, table, name, v);
    }
    for (name, f) in [
        ("startswith", startswith as crate::NativeFn),
        ("endswith", endswith),
        ("contains", str_contains),
        ("ltrim", ltrim),
        ("rtrim", rtrim),
    ] {
        let v = vm.native(name, f);
        set(vm, string, name, v);
    }
    for (name, f) in [
        ("clamp", clamp as crate::NativeFn),
        ("round", round),
        ("sign", sign),
        ("wrap", wrap),
        ("lerp", lerp),
        ("normalize", normalize),
        ("remap", remap),
        ("saturate", saturate),
        ("isnan", isnan),
        ("isinf", isinf),
        ("isfinite", isfinite),
        ("sqrt", sqrt),
        ("log", log),
        ("log10", log10),
        ("acos", acos),
        ("asin", asin),
        ("random", securerandom),
    ] {
        let v = vm.native(name, f);
        set(vm, math, name, v);
    }
    register(
        vm,
        "bit",
        &[
            ("band", band),
            ("bor", bor),
            ("bxor", bxor),
            ("bnot", bnot),
            ("lshift", lshift),
            ("rshift", rshift),
            ("arshift", arshift),
            ("mod", bit_mod),
        ],
    );
    vm.error_handler = vm.native("errorhandler", default_error_handler);
    // The client's loadstring taints its caller.
    let loadstring = vm.native("loadstring", loadstring_tainting);
    vm.set_global("loadstring", loadstring);
    // The client's string.split, string.join and string.concat are other
    // function objects than strsplit, strjoin and strconcat (string.trim is
    // strtrim).
    for (name, f) in [
        ("split", strsplit as crate::NativeFn),
        ("join", strjoin),
        ("concat", strconcat),
    ] {
        let v = vm.native(name, f);
        set(vm, string, name, v);
    }
    let issecurevalue = vm.native("issecurevalue", issecurevalue);
    vm.set_global("issecurevalue", issecurevalue);
    let compat = vm
        .load(COMPAT.as_bytes(), "=compat.lua")
        .expect("the compat script compiles");
    vm.call(compat, &[]).expect("the compat script runs");
}

fn global_table(vm: &mut Vm, name: &str) -> TableRef {
    match vm.get_global(name) {
        Value::Table(t) => t,
        _ => unreachable!("the standard library is open"),
    }
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.heap.intern(key.as_bytes());
    vm.heap.table_mut(t).set_str(k, v);
}

/// `loadstring`, which taints the caller (`LUA_LOADSTRING_TAINT`).
fn loadstring_tainting(vm: &mut Vm, c: NativeCall) -> Ret {
    let strong = vm.taint_named("*** ForceTaint_Strong ***");
    vm.taint_mut().stack = strong;
    crate::stdlib::base::loadstring(vm, c)
}

/// Argument `n` as a string (numbers convert), or the client's usage error.
fn usage_bytes(
    vm: &mut Vm,
    c: &NativeCall,
    n: usize,
    usage: &str,
) -> Result<std::rc::Rc<[u8]>, LuaError> {
    if !matches!(c.arg(n), Value::Str(_) | Value::Num(_)) {
        return Err(vm.arg_error(n, usage));
    }
    vm.check_bytes(c, n)
}

fn usage_table(vm: &mut Vm, c: &NativeCall, n: usize, usage: &str) -> Result<TableRef, LuaError> {
    match c.arg(n) {
        Value::Table(t) => Ok(t),
        _ => Err(vm.arg_error(n, usage)),
    }
}

/// `issecurevalue(v)`: whether the argument's slot is secure, and its taint.
fn issecurevalue(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = c.slot(1).t;
    Ok(match vm.taint_name(t) {
        None => vec![Value::Bool(true), Value::Nil],
        Some(name) => {
            let name = vm.str(&name);
            vec![Value::Bool(false), name]
        }
    })
}

/// A raw write from a native; a frozen table refuses it.
fn raw_set(vm: &mut Vm, t: TableRef, k: f64, v: Value) -> Result<(), LuaError> {
    match vm.heap.table_mut(t).set(Value::Num(k), v) {
        Ok(()) => Ok(()),
        Err(e) => Err(vm.rt_error(e.message())),
    }
}

fn freeze(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = usage_table(vm, &c, 1, "Usage: local frozen = table.freeze(table)")?;
    vm.heap.table_mut(t).frozen = true;
    Ok(vec![Value::Table(t)])
}

fn isfrozen(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = usage_table(vm, &c, 1, "Usage: local frozen = table.isfrozen(table)")?;
    Ok(vec![Value::Bool(vm.heap.table(t).frozen)])
}

fn gfind(vm: &mut Vm, _c: NativeCall) -> Ret {
    Err(vm.error("'string.gfind' was renamed to 'string.gmatch'"))
}

// ------------------------------------------------------------- strings

/// `strsplit(delimiters, s, limit)`: each byte of `delimiters` separates.
fn split(vm: &mut Vm, c: &NativeCall) -> Result<Vec<Value>, LuaError> {
    let delim = vm.check_bytes(c, 1)?;
    let s = vm.check_bytes(c, 2)?;
    let limit = vm.opt_int(c, 3, 0)?;
    // C strings: both end at a NUL.
    let delim = &delim[..delim.iter().position(|&b| b == 0).unwrap_or(delim.len())];
    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    let mut out = Vec::new();
    let mut i = 0;
    // `i <= end`: the terminator counts as part of the string.
    if limit == 0 || limit > 1 {
        while i <= end {
            let len = s[i..end]
                .iter()
                .position(|b| delim.contains(b))
                .unwrap_or(end - i);
            out.push(vm.bytes(&s[i..i + len]));
            i += len + 1;
            if out.len() as i64 == limit - 1 {
                break;
            }
        }
    }
    if i <= end {
        out.push(vm.bytes(&s[i..end]));
    }
    // Results go on the stack, which holds `LUAI_MAXCSTACK` (8000) slots.
    if out.len() + 3 > 8000 {
        return Err(vm.error("strsplit(): Stack overflow"));
    }
    Ok(out)
}

fn strsplit(vm: &mut Vm, c: NativeCall) -> Ret {
    split(vm, &c)
}

fn strsplittable(vm: &mut Vm, c: NativeCall) -> Ret {
    let parts = split(vm, &c)?;
    let t = vm
        .heap
        .alloc_table(crate::table::Table::new(parts.len(), 0));
    for (i, v) in parts.into_iter().enumerate() {
        let _ = vm.heap.table_mut(t).set(Value::Num((i + 1) as f64), v);
    }
    Ok(vec![Value::Table(t)])
}

fn strjoin(vm: &mut Vm, c: NativeCall) -> Ret {
    let sep = c.arg(1);
    if !matches!(sep, Value::Str(_) | Value::Num(_)) {
        return Err(vm.type_arg_error(1, "string", c.args.first().copied()));
    }
    // The pieces with the separator between them, through `lua_concat`.
    let mut parts = Vec::new();
    for (i, &v) in c.args.iter().skip(1).enumerate() {
        if i > 0 {
            parts.push(sep);
        }
        parts.push(v);
    }
    concat_values(vm, &parts)
}

/// `lua_concat` of `values`: the empty string for none.
fn concat_values(vm: &mut Vm, values: &[Value]) -> Ret {
    let Some((&first, rest)) = values.split_first() else {
        return Ok(vec![vm.str("")]);
    };
    if rest.is_empty() {
        return Ok(vec![first]);
    }
    // Right to left, as the VM concatenates.
    let mut acc = *values.last().expect("two or more");
    for &v in values[..values.len() - 1].iter().rev() {
        acc = vm.concat_values(v, acc)?;
    }
    Ok(vec![acc])
}

fn strconcat(vm: &mut Vm, c: NativeCall) -> Ret {
    concat_values(vm, &c.args)
}

fn strtrim(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = usage_bytes(
        vm,
        &c,
        1,
        "Usage: local trimmed = string.trim(str [, characters])",
    )?;
    let delim: Vec<u8> = match vm.opt_str(&c, 2)? {
        Some(d) => vm.heap.str_bytes(d).to_vec(),
        None => b" \r\n\t".to_vec(),
    };
    // `strchr` also finds the terminating NUL, and stops at an inner one.
    let delim = &delim[..delim.iter().position(|&b| b == 0).unwrap_or(delim.len())];
    let is_delim = |b: u8| b == 0 || delim.contains(&b);
    let mut begin = 0;
    while begin < s.len() && is_delim(s[begin]) {
        begin += 1;
    }
    // The end scan starts at the terminator, as elune's does.
    let mut end = s.len() as isize;
    while end >= begin as isize
        && is_delim(if end as usize == s.len() {
            0
        } else {
            s[end as usize]
        })
    {
        end -= 1;
    }
    let out = if end < begin as isize {
        &[][..]
    } else {
        &s[begin..=end as usize]
    };
    Ok(vec![vm.bytes(out)])
}

/// Code points, counting each byte that does not start a sequence as one
/// (`utf8len`).
fn utf8_len(s: &[u8]) -> usize {
    let mut i = 0;
    let mut n = 0;
    while i < s.len() && s[i] != 0 {
        let c = s[i];
        i += if c & 0xf8 == 0xf0 {
            4
        } else if c & 0xf0 == 0xe0 {
            3
        } else if c & 0xe0 == 0xc0 {
            2
        } else {
            1
        };
        n += 1;
    }
    n
}

fn strlenutf8(vm: &mut Vm, c: NativeCall) -> Ret {
    if !matches!(c.arg(1), Value::Str(_) | Value::Num(_)) {
        return Err(vm.error("Usage: strlenutf8(string)"));
    }
    let s = vm.check_bytes(&c, 1)?;
    Ok(vec![Value::Num(utf8_len(&s) as f64)])
}

/// The code points of `s` (up to a NUL), uppercased.
fn upper_code_points(s: &[u8]) -> Vec<u32> {
    let s = &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())];
    String::from_utf8_lossy(s)
        .chars()
        .map(|ch| {
            let mut up = ch.to_uppercase();
            match (up.next(), up.next()) {
                (Some(u), None) => u as u32,
                _ => ch as u32,
            }
        })
        .collect()
}

/// The difference of the first unequal (uppercased) code points; not
/// clamped (`strcmputf8i("a", "ab")` is -66).
fn strcmputf8i(vm: &mut Vm, c: NativeCall) -> Ret {
    let a = vm.check_bytes(&c, 1)?;
    let b = vm.check_bytes(&c, 2)?;
    let (a, b) = (upper_code_points(&a), upper_code_points(&b));
    let mut i = 0;
    let r = loop {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y || x == 0 {
            break i64::from(x) - i64::from(y);
        }
        i += 1;
    };
    Ok(vec![Value::Num(r as f64)])
}

// -------------------------------------------------------------- tables

fn wipe(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let keys: Vec<Value> = vm.heap.table(t).entries().map(|(k, _)| k).collect();
    for k in keys {
        // `lua_settable`: metamethods run for each key.
        vm.set_index(Value::Table(t), k, Value::Nil, None)?;
    }
    Ok(vec![Value::Table(t)])
}

fn removemulti(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let length = vm.heap.table(t).border() as i64;
    let index = vm.opt_int(&c, 2, length)?;
    let count = vm.opt_int(&c, 3, 1)?;
    if length == 0 {
        return Ok(vec![]);
    }
    if index <= 0 || count < 0 || index + count - 1 > length {
        return Err(vm.error("parameters out of bounds"));
    }
    let mut removed = Vec::new();
    for dst in index..=length {
        let src = dst + count;
        if dst < index + count {
            removed.push(vm.heap.table(t).get_num(dst as f64));
        }
        let v = if src <= length {
            vm.heap.table(t).get_num(src as f64)
        } else {
            Value::Nil
        };
        raw_set(vm, t, dst as f64, v)?;
    }
    Ok(removed)
}

fn count(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = usage_table(vm, &c, 1, "Usage: local count = table.count(table)")?;
    Ok(vec![Value::Num(vm.heap.table(t).entries().count() as f64)])
}

/// Entries, then the sizes of the array and hash parts.
fn getcountinfo(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let table = vm.heap.table(t);
    let (array, hash) = table.part_sizes();
    Ok(vec![
        Value::Num(table.entries().count() as f64),
        Value::Num(array as f64),
        Value::Num(hash as f64),
    ])
}

fn create(vm: &mut Vm, c: NativeCall) -> Ret {
    const USAGE: &str = "Usage: local table = table.create(arraySizeHint [, nodeSizeHint])";
    let narr = usage_num(vm, &c, 1, USAGE)?.max(0.0) as usize;
    let nhash = if c.arg(2).is_nil() {
        0
    } else {
        usage_num(vm, &c, 2, USAGE)?.max(0.0) as usize
    };
    let t = vm.heap.alloc_table(crate::table::Table::new(narr, nhash));
    Ok(vec![Value::Table(t)])
}

/// The first array index (1..#t) holding `value`.
fn find_index(vm: &mut Vm, t: TableRef, v: Value) -> Option<usize> {
    let n = vm.heap.table(t).border();
    (1..=n).find(|&i| vm.heap.table(t).get_num(i as f64).raw_eq(v))
}

/// Whether any value of the table (any key) is `value`.
fn contains(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = usage_table(
        vm,
        &c,
        1,
        "Usage: local contains = table.contains(table, value)",
    )?;
    let v = c.arg(2);
    let found = vm.heap.table(t).entries().any(|(_, x)| x.raw_eq(v));
    Ok(vec![Value::Bool(found)])
}

fn indexof(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = usage_table(
        vm,
        &c,
        1,
        "Usage: local index = table.indexof(table [, value])",
    )?;
    Ok(vec![
        find_index(vm, t, c.arg(2)).map_or(Value::Nil, |i| Value::Num(i as f64)),
    ])
}

fn isempty(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    Ok(vec![Value::Bool(
        vm.heap.table(t).entries().next().is_none(),
    )])
}

fn list(vm: &mut Vm, items: Vec<Value>) -> Value {
    let t = vm
        .heap
        .alloc_table(crate::table::Table::new(items.len(), 0));
    for (i, v) in items.into_iter().enumerate() {
        let _ = vm.heap.table_mut(t).set(Value::Num((i + 1) as f64), v);
    }
    Value::Table(t)
}

fn keys(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let items: Vec<Value> = vm.heap.table(t).entries().map(|(k, _)| k).collect();
    Ok(vec![list(vm, items)])
}

fn values(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let items: Vec<Value> = vm.heap.table(t).entries().map(|(_, v)| v).collect();
    Ok(vec![list(vm, items)])
}

/// Removes every occurrence from the array part; returns how many.
fn removevalue(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let v = c.arg(2);
    let n = vm.heap.table(t).border();
    let kept: Vec<Value> = (1..=n)
        .map(|i| vm.heap.table(t).get_num(i as f64))
        .filter(|x| !x.raw_eq(v))
        .collect();
    let removed = n - kept.len();
    for i in 1..=n {
        let x = kept.get(i - 1).copied().unwrap_or(Value::Nil);
        raw_set(vm, t, i as f64, x)?;
    }
    Ok(vec![Value::Num(removed as f64)])
}

/// Removes `t[i]` by moving the last element there.
fn removeunordered(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let i = vm.check_int(&c, 2)?;
    let n = vm.heap.table(t).border() as i64;
    if i < 1 || i > n {
        return Ok(vec![Value::Nil]);
    }
    let removed = vm.heap.table(t).get_num(i as f64);
    let last = vm.heap.table(t).get_num(n as f64);
    raw_set(vm, t, i as f64, last)?;
    raw_set(vm, t, n as f64, Value::Nil)?;
    Ok(vec![removed])
}

// ----------------------------------------------------- string additions

fn startswith(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = usage_bytes(
        vm,
        &c,
        1,
        "Usage: local startsWith = string.startswith(str, prefix)",
    )?;
    let p = usage_bytes(
        vm,
        &c,
        2,
        "Usage: local startsWith = string.startswith(str, prefix)",
    )?;
    Ok(vec![Value::Bool(s.starts_with(&p))])
}

fn endswith(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = usage_bytes(
        vm,
        &c,
        1,
        "Usage: local endsWith = string.endswith(str, suffix)",
    )?;
    let p = usage_bytes(
        vm,
        &c,
        2,
        "Usage: local endsWith = string.endswith(str, suffix)",
    )?;
    Ok(vec![Value::Bool(s.ends_with(&p))])
}

fn str_contains(vm: &mut Vm, c: NativeCall) -> Ret {
    const USAGE: &str = "Usage: local contains = string.contains(str, substring)";
    let s = usage_bytes(vm, &c, 1, USAGE)?;
    let p = usage_bytes(vm, &c, 2, USAGE)?;
    let found = p.is_empty() || s.windows(p.len()).any(|w| w == &p[..]);
    Ok(vec![Value::Bool(found)])
}

fn trim_set(vm: &mut Vm, c: &NativeCall) -> Result<Vec<u8>, LuaError> {
    Ok(match vm.opt_str(c, 2)? {
        Some(d) => vm.heap.str_bytes(d).to_vec(),
        None => b" \r\n\t".to_vec(),
    })
}

fn ltrim(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let set = trim_set(vm, &c)?;
    let start = s.iter().position(|b| !set.contains(b)).unwrap_or(s.len());
    Ok(vec![vm.bytes(&s[start..])])
}

fn rtrim(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let set = trim_set(vm, &c)?;
    let end = s
        .iter()
        .rposition(|b| !set.contains(b))
        .map_or(0, |i| i + 1);
    Ok(vec![vm.bytes(&s[..end])])
}

// ------------------------------------------------------- math additions

/// Argument `n` as a number, or the client's usage error for the function.
fn usage_num(vm: &mut Vm, c: &NativeCall, n: usize, usage: &str) -> Result<f64, LuaError> {
    match vm.tonumber(c.arg(n)) {
        Some(x) => Ok(x),
        None => Err(vm.arg_error(n, usage)),
    }
}

fn num(x: f64) -> Ret {
    Ok(vec![Value::Num(x)])
}

/// C's `round`: halves away from zero.
fn round_half_away(x: f64) -> f64 {
    if x >= 0.0 {
        (x + 0.5).floor()
    } else {
        -(-x + 0.5).floor()
    }
}

fn clamp(vm: &mut Vm, c: NativeCall) -> Ret {
    const USAGE: &str = "Usage: local clampedValue = math.clamp(value, minimum, maximum)";
    let v = usage_num(vm, &c, 1, USAGE)?;
    let lo = usage_num(vm, &c, 2, USAGE)?;
    let hi = usage_num(vm, &c, 3, USAGE)?;
    // `fmin(fmax(value, minimum), maximum)`: NaN gives the minimum.
    num(v.max(lo).min(hi))
}

fn round(vm: &mut Vm, c: NativeCall) -> Ret {
    const USAGE: &str = "Usage: local roundedValue = math.round(value [, decimalPlaces])";
    let v = usage_num(vm, &c, 1, USAGE)?;
    let d = if c.arg(2).is_nil() {
        0.0
    } else {
        usage_num(vm, &c, 2, USAGE)?
    };
    let m = 10f64.powf(d);
    num(round_half_away(v * m) / m)
}

fn sign(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = usage_num(vm, &c, 1, "Usage: local sign = math.sign(value)")?;
    // NaN counts as negative.
    num(if v > 0.0 {
        1.0
    } else if v == 0.0 {
        0.0
    } else {
        -1.0
    })
}

fn wrap(vm: &mut Vm, c: NativeCall) -> Ret {
    const USAGE: &str = "Usage: local wrappedValue = math.wrap(value, minimum, maximum)";
    let v = usage_num(vm, &c, 1, USAGE)?;
    let lo = usage_num(vm, &c, 2, USAGE)?;
    let hi = usage_num(vm, &c, 3, USAGE)?;
    // An empty or reversed range gives the minimum.
    if hi <= lo {
        return num(lo);
    }
    num(lo + (v - lo).rem_euclid(hi - lo))
}

fn lerp(vm: &mut Vm, c: NativeCall) -> Ret {
    const USAGE: &str = "Usage: local value = math.lerp(startValue, endValue, amount)";
    let a = usage_num(vm, &c, 1, USAGE)?;
    let b = usage_num(vm, &c, 2, USAGE)?;
    let t = usage_num(vm, &c, 3, USAGE)?;
    num(a + (b - a) * t)
}

/// `(value - min) / (max - min)`; an empty range gives 0.
fn normalized(v: f64, lo: f64, hi: f64) -> f64 {
    if hi == lo { 0.0 } else { (v - lo) / (hi - lo) }
}

fn normalize(vm: &mut Vm, c: NativeCall) -> Ret {
    const USAGE: &str = "Usage: local normalizedValue = math.normalize(value, minimum, maximum)";
    let v = usage_num(vm, &c, 1, USAGE)?;
    let lo = usage_num(vm, &c, 2, USAGE)?;
    let hi = usage_num(vm, &c, 3, USAGE)?;
    num(normalized(v, lo, hi))
}

fn remap(vm: &mut Vm, c: NativeCall) -> Ret {
    const USAGE: &str = "Usage: local remappedValue = math.remap(value, fromMinimum, fromMaximum, toMinimum, toMaximum)";
    let v = usage_num(vm, &c, 1, USAGE)?;
    let a = usage_num(vm, &c, 2, USAGE)?;
    let b = usage_num(vm, &c, 3, USAGE)?;
    let x = usage_num(vm, &c, 4, USAGE)?;
    let y = usage_num(vm, &c, 5, USAGE)?;
    num(x + (y - x) * normalized(v, a, b))
}

fn saturate(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = usage_num(
        vm,
        &c,
        1,
        "Usage: local saturatedValue = math.saturate(value)",
    )?;
    // fmax/fmin, as C: NaN gives 0 (Rust's clamp would keep NaN).
    #[allow(clippy::manual_clamp)]
    num(v.max(0.0).min(1.0))
}

fn isnan(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = usage_num(vm, &c, 1, "Usage: local nan = math.isnan(value)")?;
    Ok(vec![Value::Bool(v.is_nan())])
}

fn isinf(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = usage_num(vm, &c, 1, "Usage: local inf = math.isinf(value)")?;
    Ok(vec![Value::Bool(v.is_infinite())])
}

fn isfinite(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = usage_num(vm, &c, 1, "Usage: local finite = math.isfinite(value)")?;
    Ok(vec![Value::Bool(v.is_finite())])
}

/// A number from `[lo, hi]`'s checks: domain errors as the client words them.
fn domain(vm: &mut Vm, c: &NativeCall, ok: fn(f64) -> bool, what: &str) -> Result<f64, LuaError> {
    let x = vm.check_number(c, 1)?;
    if !ok(x) {
        let got = crate::number::fmt_num(x);
        return Err(vm.arg_error(1, &format!("expected {what}, got {got}")));
    }
    Ok(x)
}

fn sqrt(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = domain(vm, &c, |x| x >= 0.0 || x.is_nan(), "non-negative argument")?;
    num(x.sqrt())
}

fn log(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = domain(vm, &c, |x| x >= 0.0 || x.is_nan(), "non-negative argument")?;
    num(x.ln())
}

fn log10(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = domain(vm, &c, |x| x >= 0.0 || x.is_nan(), "non-negative argument")?;
    num(x.log10())
}

fn acos(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = domain(
        vm,
        &c,
        |x| (-1.0..=1.0).contains(&x),
        "argument between (-1, 1)",
    )?;
    num(x.acos())
}

fn asin(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = domain(
        vm,
        &c,
        |x| (-1.0..=1.0).contains(&x),
        "argument between (-1, 1)",
    )?;
    num(x.asin())
}

/// `math.random` is the client's `securerandom`.
fn securerandom(vm: &mut Vm, c: NativeCall) -> Ret {
    if c.len() > 2 {
        return Err(
            vm.error("Usage: securerandom(); securerandom(highint); securerandom(lowint, highint)")
        );
    }
    random_range(vm, c)
}

/// `[0, 1)`, `[1, high]` or `[low, high]`; an empty interval is argument 1's
/// fault.
fn random_range(vm: &mut Vm, c: NativeCall) -> Ret {
    let r = f64::from(vm.rand() % 0x7fff_ffff) / f64::from(0x7fff_ffffu32);
    match c.len() {
        0 => num(r),
        1 => {
            let u = vm.check_int(&c, 1)?;
            if u < 1 {
                return Err(vm.arg_error(1, "interval is empty"));
            }
            num((r * u as f64).floor() + 1.0)
        }
        _ => {
            let l = vm.check_int(&c, 1)?;
            let u = vm.check_int(&c, 2)?;
            if l > u {
                return Err(vm.arg_error(1, "interval is empty"));
            }
            num((r * (u - l + 1) as f64).floor() + l as f64)
        }
    }
}

// ----------------------------------------------------------------- bit

/// A number as 32 bits: truncated, then wrapped (the client raises no
/// overflow here: `bit.band(2 ^ 32 + 1, 1)` is 1).
fn unsigned(vm: &mut Vm, c: &NativeCall, n: usize) -> Result<u32, LuaError> {
    let x = vm.check_number(c, n)?;
    Ok(if x.is_finite() {
        (x.trunc() % 4_294_967_296.0) as i64 as u32
    } else {
        0
    })
}

fn signed(vm: &mut Vm, c: &NativeCall, n: usize) -> Result<i32, LuaError> {
    Ok(unsigned(vm, c, n)? as i32)
}

fn fold(vm: &mut Vm, c: NativeCall, f: fn(u32, u32) -> u32) -> Ret {
    let mut r = unsigned(vm, &c, 1)?;
    for i in 2..=c.len() {
        r = f(r, unsigned(vm, &c, i)?);
    }
    Ok(vec![Value::Num(f64::from(r))])
}

fn band(vm: &mut Vm, c: NativeCall) -> Ret {
    fold(vm, c, |a, b| a & b)
}

fn bor(vm: &mut Vm, c: NativeCall) -> Ret {
    fold(vm, c, |a, b| a | b)
}

fn bxor(vm: &mut Vm, c: NativeCall) -> Ret {
    fold(vm, c, |a, b| a ^ b)
}

fn bnot(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = unsigned(vm, &c, 1)?;
    Ok(vec![Value::Num(f64::from(!x))])
}

// Shift counts are taken modulo 32 (`bit.lshift(1, 32)` is 1).
fn lshift(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = unsigned(vm, &c, 1)?;
    let n = unsigned(vm, &c, 2)?;
    Ok(vec![Value::Num(f64::from(x << (n & 31)))])
}

fn rshift(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = unsigned(vm, &c, 1)?;
    let n = unsigned(vm, &c, 2)?;
    Ok(vec![Value::Num(f64::from(x >> (n & 31)))])
}

fn arshift(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = signed(vm, &c, 1)?;
    let n = unsigned(vm, &c, 2)?;
    Ok(vec![Value::Num(f64::from(x >> (n & 31)))])
}

fn bit_mod(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = signed(vm, &c, 1)?;
    let y = signed(vm, &c, 2)?;
    // Division by zero traps on x86 and gives 0 on ARM.
    Ok(vec![Value::Num(f64::from(x.checked_rem(y).unwrap_or(0)))])
}

// ---------------------------------------------------------------- misc

/// `pcallwithenv(f, env, ...)`: `pcall` with `f`'s environment replaced
/// for the call; a caller's taint taints `f` (elune's `luaB_pcallwithenv`).
fn pcallwithenv(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Func(f), Value::Table(env)) = (c.arg(1), c.arg(2)) else {
        return Err(vm.error("Usage: local success, ... = pcallwithenv(f, env, [args...])"));
    };
    let old = vm.func_env(f);
    if let Some(meta) = vm.heap.table(old).meta {
        let k = vm.heap.intern(b"__environment");
        if !vm.heap.table(meta).get_str(k).is_nil() {
            return Err(
                vm.error("cannot use pcallwithenv on a function with a protected environment")
            );
        }
    }
    let taint_object = |vm: &mut Vm| {
        let w = vm.taint().write();
        if !w.is_secure() {
            vm.heap.func_mut(f).set_taint(w);
        }
    };
    taint_object(vm);
    vm.set_func_env(f, env);
    let saved = vm.errfunc.take();
    let r = vm.call_slots(c.slot(1), &c.slots(3));
    vm.errfunc = saved;
    vm.set_func_env(f, old);
    taint_object(vm);
    let out = match r {
        Ok(mut results) => {
            results.insert(0, vm.taint().fresh(Value::Bool(true)));
            results
        }
        Err(e) => vec![
            vm.taint().fresh(Value::Bool(false)),
            vm.taint().fresh(e.value),
        ],
    };
    Ok(vm.return_slots(out))
}

fn geterrorhandler(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.error_handler])
}

fn seterrorhandler(vm: &mut Vm, c: NativeCall) -> Ret {
    let f = vm.check_func(&c, 1)?;
    vm.error_handler = f;
    Ok(vec![])
}

fn fastrandom(vm: &mut Vm, c: NativeCall) -> Ret {
    random_range(vm, c)
}

// ---------------------------------------------------------------- time

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `struct tm` of seconds since the epoch, in UTC.
pub(crate) struct Tm {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub min: i64,
    pub sec: i64,
    pub wday: i64,
    pub yday: i64,
}

fn gmtime(t: i64) -> Tm {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Tm {
        year,
        month,
        day,
        hour: secs / 3600,
        min: secs % 3600 / 60,
        sec: secs % 60,
        wday: (days + 4).rem_euclid(7),
        yday: days - days_from_civil(year, 1, 1),
    }
}

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// One `strftime` conversion in the C locale (as macOS formats it).
fn strftime(conv: u8, tm: &Tm, out: &mut Vec<u8>) {
    let weekday = WEEKDAYS[tm.wday as usize];
    let month = MONTHS[(tm.month - 1) as usize];
    let hour12 = if tm.hour % 12 == 0 { 12 } else { tm.hour % 12 };
    let s = match conv {
        b'a' => weekday[..3].to_string(),
        b'A' => weekday.to_string(),
        b'b' | b'h' => month[..3].to_string(),
        b'B' => month.to_string(),
        b'c' => format!(
            "{} {} {:2} {:02}:{:02}:{:02} {}",
            &weekday[..3],
            &month[..3],
            tm.day,
            tm.hour,
            tm.min,
            tm.sec,
            tm.year
        ),
        b'd' => format!("{:02}", tm.day),
        b'e' => format!("{:2}", tm.day),
        b'H' => format!("{:02}", tm.hour),
        b'I' => format!("{hour12:02}"),
        b'j' => format!("{:03}", tm.yday + 1),
        b'm' => format!("{:02}", tm.month),
        b'M' => format!("{:02}", tm.min),
        b'p' => (if tm.hour < 12 { "AM" } else { "PM" }).to_string(),
        b'S' => format!("{:02}", tm.sec),
        b'w' => tm.wday.to_string(),
        b'x' => format!(
            "{:02}/{:02}/{:02}",
            tm.month,
            tm.day,
            tm.year.rem_euclid(100)
        ),
        b'X' => format!("{:02}:{:02}:{:02}", tm.hour, tm.min, tm.sec),
        b'y' => format!("{:02}", tm.year.rem_euclid(100)),
        b'Y' => tm.year.to_string(),
        b'Z' => "UTC".to_string(),
        b'%' => "%".to_string(),
        // Unknown conversions: macOS writes the character.
        other => (other as char).to_string(),
    };
    out.extend_from_slice(s.as_bytes());
}

fn date(vm: &mut Vm, c: NativeCall) -> Ret {
    let fmt = match vm.opt_str(&c, 1)? {
        Some(s) => vm.heap.str_bytes(s).to_vec(),
        None => b"%c".to_vec(),
    };
    let t = if c.arg(2).is_nil() {
        vm.clock.now.floor()
    } else {
        vm.check_number(&c, 2)?
    };
    // The VM's local time is UTC plus its offset; `!` is UTC.
    let (fmt, offset) = match fmt.strip_prefix(b"!") {
        Some(rest) => (rest.to_vec(), 0),
        None => (fmt, vm.clock.utc_offset),
    };
    let tm = gmtime(t as i64 + offset);
    if fmt == b"*t" {
        let r = vm.new_table();
        for (k, v) in [
            ("sec", tm.sec),
            ("min", tm.min),
            ("hour", tm.hour),
            ("day", tm.day),
            ("month", tm.month),
            ("year", tm.year),
            ("wday", tm.wday + 1),
            ("yday", tm.yday + 1),
        ] {
            set(vm, r, k, Value::Num(v as f64));
        }
        set(vm, r, "isdst", Value::Bool(false));
        return Ok(vec![Value::Table(r)]);
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' || i + 1 == fmt.len() {
            out.push(fmt[i]);
        } else {
            i += 1;
            strftime(fmt[i], &tm, &mut out);
        }
        i += 1;
    }
    Ok(vec![vm.bytes(&out)])
}

/// A date table's field (`getfield`).
fn field(vm: &mut Vm, t: TableRef, key: &str, default: Option<i64>) -> Result<i64, LuaError> {
    let k = vm.str(key);
    let v = vm.heap.table(t).get(k);
    match vm.tonumber(v) {
        Some(n) => Ok(n as i64),
        None => match default {
            Some(d) => Ok(d),
            None => Err(vm.error(&format!("field '{key}' missing in date table"))),
        },
    }
}

fn time(vm: &mut Vm, c: NativeCall) -> Ret {
    if c.arg(1).is_nil() {
        return Ok(vec![Value::Num(vm.clock.now.floor())]);
    }
    let t = vm.check_table(&c, 1)?;
    let sec = field(vm, t, "sec", Some(0))?;
    let min = field(vm, t, "min", Some(0))?;
    let hour = field(vm, t, "hour", Some(12))?;
    let day = field(vm, t, "day", None)?;
    let month = field(vm, t, "month", None)?;
    let year = field(vm, t, "year", None)?;
    // `mktime` normalizes out-of-range fields.
    let (year, month) = (
        year + (month - 1).div_euclid(12),
        (month - 1).rem_euclid(12) + 1,
    );
    let days = days_from_civil(year, month, 1) + day - 1;
    let local = days * 86_400 + hour * 3600 + min * 60 + sec;
    Ok(vec![Value::Num((local - vm.clock.utc_offset) as f64)])
}

fn difftime(vm: &mut Vm, c: NativeCall) -> Ret {
    let a = vm.check_number(&c, 1)?.trunc();
    let b = vm.opt_number(&c, 2, 0.0)?.trunc();
    Ok(vec![Value::Num(a - b)])
}

fn debugprofilestart(vm: &mut Vm, _c: NativeCall) -> Ret {
    vm.clock.profile_start = vm.clock.elapsed_ms;
    Ok(vec![])
}

fn debugprofilestop(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(
        vm.clock.elapsed_ms - vm.clock.profile_start,
    )])
}

/// The default error handler: the message, as it is.
fn default_error_handler(_vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![c.arg(1)])
}

// --------------------------------------------------------------- debug

/// `debugstack([thread,] [start [, top [, base]]])`: Lua 5.0's traceback,
/// `top` levels from `start`, then `...`, then the last `base` levels
/// (elune's `db_stack`).
fn debugstack(vm: &mut Vm, c: NativeCall) -> Ret {
    // Nothing inside `__gc` metamethods (patch 10.1).
    if vm.in_finalizer {
        return Ok(vec![]);
    }
    let mut args: &[Value] = &c.args;
    let thread = match args.first() {
        Some(Value::Thread(t)) => {
            args = &args[1..];
            Some(*t)
        }
        _ => None,
    };
    let mut numbers = [1i64, 12, 10];
    for slot in &mut numbers {
        match args.first().map(|&v| (v, vm.tonumber(v))) {
            Some((Value::Num(_) | Value::Str(_), Some(n))) => {
                *slot = n as i64;
                args = &args[1..];
            }
            _ => break,
        }
    }
    let [start, ntop, nbase] = numbers;
    // What is left: a string prefix, or a value that is returned as it is.
    let prefix = match args.first() {
        None => Vec::new(),
        Some(&v @ (Value::Str(_) | Value::Num(_))) => {
            let mut b = Vec::new();
            vm.append_strlike(&mut b, v);
            b
        }
        Some(_) => return Ok(vec![Value::Nil]),
    };
    let levels = match thread {
        Some(t) if t != vm.current => vm.thread_levels(t),
        _ => vm.levels(),
    };
    let n = levels.len() as i64;
    let exists = |l: i64| (0..n).contains(&l);
    let mut out = prefix;
    let mut level = start;
    let mut firstpart = true;
    while exists(level) {
        let cur = level;
        level += 1;
        if ntop + start < level && firstpart {
            if !exists(level + nbase) {
                level -= 1;
            } else {
                out.extend_from_slice(b"...");
                while exists(level + nbase) {
                    level += 1;
                }
            }
            firstpart = false;
            continue;
        }
        let info = &levels[cur as usize];
        // The client's format: `[source]:line: in function 'name'`, the
        // source without its `@`; `<short source:line>` for unnamed ones.
        match info.what {
            "C" => out.extend_from_slice(b"[C]:"),
            "tail" => out.extend_from_slice(b"[tail call]:"),
            _ => {
                out.push(b'[');
                out.extend_from_slice(info.source.strip_prefix(b"@").unwrap_or(&info.source));
                out.extend_from_slice(b"]:");
            }
        }
        if info.currentline > 0 {
            out.extend_from_slice(format!("{}:", info.currentline).as_bytes());
        }
        match &info.name {
            Some((kind, name)) if matches!(*kind, "global" | "local" | "field" | "method") => {
                out.extend_from_slice(format!(" in function '{name}'").as_bytes());
            }
            _ => match info.what {
                "main" => out.extend_from_slice(b" in main chunk"),
                "C" | "tail" => out.extend_from_slice(b" ?"),
                _ => {
                    out.extend_from_slice(b" in function <");
                    out.extend_from_slice(&info.short_src);
                    out.extend_from_slice(format!(":{}>", info.linedefined).as_bytes());
                }
            },
        }
        out.push(b'\n');
    }
    Ok(vec![vm.bytes(&out)])
}

/// `debuglocals([level])`: the locals and upvalues of a level, dumped
/// (elune's `db_locals`).
fn debuglocals(vm: &mut Vm, c: NativeCall) -> Ret {
    if vm.in_finalizer {
        return Ok(vec![]);
    }
    let level = (vm.opt_number(&c, 1, 1.0)? as i64).max(1);
    let levels = vm.levels();
    let mut out = Vec::new();
    if let Some(info) = levels.get(level as usize) {
        let (locals, upvalues) = vm.level_variables(info);
        for (name, v) in locals.into_iter().chain(upvalues) {
            dump_value(vm, &name, v, 1, &mut out);
        }
    }
    Ok(vec![vm.bytes(&out)])
}

/// One variable of `debuglocals`, as the client prints it: `name=value`.
fn dump_value(vm: &mut Vm, name: &str, v: Value, _recurse: u32, out: &mut Vec<u8>) {
    out.extend_from_slice(name.as_bytes());
    out.push(b'=');
    match v {
        Value::Nil => out.extend_from_slice(b"nil"),
        Value::Bool(b) => out.extend_from_slice(if b { b"true" } else { b"false" }),
        Value::Num(n) => out.extend_from_slice(crate::number::fmt_num(n).as_bytes()),
        Value::Str(s) => {
            out.push(b'"');
            out.extend_from_slice(vm.str_bytes(s));
            out.push(b'"');
        }
        other => out.extend_from_slice(format!("<{}>", other.type_name()).as_bytes()),
    }
    out.push(b'\n');
}
