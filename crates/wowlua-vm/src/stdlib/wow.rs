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
string.split = strsplit
string.join = strjoin
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
    };
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
        ("tostringall", tostringall),
        ("nop", nop),
        ("assertf", assertf),
        ("getglobal", getglobal),
        ("setglobal", setglobal),
        ("ipairs_reverse", ipairs_reverse),
        ("pcallwithenv", pcallwithenv),
        ("geterrorhandler", geterrorhandler),
        ("seterrorhandler", seterrorhandler),
        ("getprinthandler", getprinthandler),
        ("setprinthandler", setprinthandler),
        ("print", print),
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
        ("rtgsub", rtgsub),
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
    vm.print_handler = vm.native("print", crate::stdlib::base::print);
    vm.error_handler = vm.native("errorhandler", default_error_handler);
    let strconcat = vm.get_global("strconcat");
    set(vm, string, "concat", strconcat);
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
    let sep = vm.check_bytes(&c, 1)?;
    let pieces = c.len().saturating_sub(1);
    if pieces <= 1 || sep.is_empty() {
        // `lua_concat`: strings and numbers, else the concat metamethods.
        return concat_values(vm, &c.args[1.min(c.len())..]);
    }
    let mut out = Vec::new();
    for i in 1..=pieces {
        let s = vm.check_bytes(&c, i + 1)?;
        // `luaL_addstring` stops at a NUL.
        out.extend_from_slice(&s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]);
        if i < pieces {
            out.extend_from_slice(&sep);
        }
    }
    Ok(vec![vm.bytes(&out)])
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
    let s = vm.check_bytes(&c, 1)?;
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
    let s = vm.check_bytes(&c, 1)?;
    Ok(vec![Value::Num(utf8_len(&s) as f64)])
}

/// The code points of `s` (up to a NUL), lowercased.
fn lower_code_points(s: &[u8]) -> Vec<u32> {
    let s = &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())];
    String::from_utf8_lossy(s)
        .chars()
        .map(|ch| ch.to_lowercase().next().map_or(ch as u32, |l| l as u32))
        .collect()
}

fn strcmputf8i(vm: &mut Vm, c: NativeCall) -> Ret {
    let a = vm.check_bytes(&c, 1)?;
    let b = vm.check_bytes(&c, 2)?;
    let (a, b) = (lower_code_points(&a), lower_code_points(&b));
    let mut i = 0;
    let r = loop {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y || x == 0 {
            break i64::from(x) - i64::from(y);
        }
        i += 1;
    };
    Ok(vec![Value::Num(r.clamp(-1, 1) as f64)])
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
        let _ = vm.heap.table_mut(t).set(Value::Num(dst as f64), v);
    }
    Ok(removed)
}

fn count(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
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
    let narr = vm.check_int(&c, 1)?.max(0) as usize;
    let nhash = vm.opt_int(&c, 2, 0)?.max(0) as usize;
    let t = vm.heap.alloc_table(crate::table::Table::new(narr, nhash));
    Ok(vec![Value::Table(t)])
}

/// The array part's index of `value` from `start`.
fn find_index(vm: &mut Vm, c: &NativeCall) -> Result<Option<usize>, LuaError> {
    let t = vm.check_table(c, 1)?;
    let v = c.arg(2);
    let start = vm.opt_int(c, 3, 1)?.max(1) as usize;
    let n = vm.heap.table(t).border();
    Ok((start..=n).find(|&i| vm.heap.table(t).get_num(i as f64).raw_eq(v)))
}

fn contains(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(find_index(vm, &c)?.is_some())])
}

fn indexof(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![
        find_index(vm, &c)?.map_or(Value::Nil, |i| Value::Num(i as f64)),
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
        let _ = vm.heap.table_mut(t).set(Value::Num(i as f64), x);
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
    let _ = vm.heap.table_mut(t).set(Value::Num(i as f64), last);
    let _ = vm.heap.table_mut(t).set(Value::Num(n as f64), Value::Nil);
    Ok(vec![removed])
}

// ----------------------------------------------------- string additions

fn startswith(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let p = vm.check_bytes(&c, 2)?;
    Ok(vec![Value::Bool(s.starts_with(&p))])
}

fn endswith(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let p = vm.check_bytes(&c, 2)?;
    Ok(vec![Value::Bool(s.ends_with(&p))])
}

fn str_contains(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let p = vm.check_bytes(&c, 2)?;
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

/// `gsub` that replaces the last `n` matches.
fn rtgsub(vm: &mut Vm, c: NativeCall) -> Ret {
    let gsub = match vm.get_global("string") {
        Value::Table(t) => {
            let k = vm.str("gsub");
            vm.heap.table(t).get(k)
        }
        _ => Value::Nil,
    };
    if c.arg(4).is_nil() {
        return vm.call(gsub, &c.args[..c.len().min(3)]);
    }
    // Count the matches, then skip all but the last `n`.
    let n = vm.check_int(&c, 4)?;
    let all = vm.call(gsub, &[c.arg(1), c.arg(2), c.arg(2)])?;
    let total = vm
        .tonumber(all.get(1).copied().unwrap_or_default())
        .unwrap_or(0.0) as i64;
    let skip = (total - n).max(0);
    let script = vm
        .load(
            b"local s, p, r, skip = ... local i = 0 return string.gsub(s, p, function(...) i = i + 1 if i <= skip then return nil end if type(r) == 'function' then return r(...) elseif type(r) == 'table' then return r[...] end return (string.gsub(..., '.*', r)) end)",
            "=rtgsub",
        )
        .expect("compiles");
    vm.call(
        script,
        &[c.arg(1), c.arg(2), c.arg(3), Value::Num(skip as f64)],
    )
}

// ------------------------------------------------------- math additions

fn nums<const N: usize>(vm: &mut Vm, c: &NativeCall) -> Result<[f64; N], LuaError> {
    let mut out = [0.0; N];
    for (i, x) in out.iter_mut().enumerate() {
        *x = vm.check_number(c, i + 1)?;
    }
    Ok(out)
}

fn num(x: f64) -> Ret {
    Ok(vec![Value::Num(x)])
}

fn clamp(vm: &mut Vm, c: NativeCall) -> Ret {
    let [v, lo, hi] = nums(vm, &c)?;
    num(if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    })
}

fn round(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = vm.check_number(&c, 1)?;
    let d = vm.opt_number(&c, 2, 0.0)?;
    let m = 10f64.powf(d);
    num((v * m + 0.5).floor() / m)
}

fn sign(vm: &mut Vm, c: NativeCall) -> Ret {
    let [v] = nums(vm, &c)?;
    num(if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        0.0
    })
}

fn wrap(vm: &mut Vm, c: NativeCall) -> Ret {
    let [v, lo, hi] = nums(vm, &c)?;
    let range = hi - lo;
    num(lo + (v - lo).rem_euclid(range))
}

fn lerp(vm: &mut Vm, c: NativeCall) -> Ret {
    let [a, b, t] = nums(vm, &c)?;
    num(a + (b - a) * t)
}

fn normalize(vm: &mut Vm, c: NativeCall) -> Ret {
    let [v, lo, hi] = nums(vm, &c)?;
    num((v - lo) / (hi - lo))
}

fn remap(vm: &mut Vm, c: NativeCall) -> Ret {
    let [v, a, b, x, y] = nums(vm, &c)?;
    num(x + (v - a) / (b - a) * (y - x))
}

fn saturate(vm: &mut Vm, c: NativeCall) -> Ret {
    let [v] = nums(vm, &c)?;
    num(v.clamp(0.0, 1.0))
}

fn isnan(vm: &mut Vm, c: NativeCall) -> Ret {
    let [v] = nums(vm, &c)?;
    Ok(vec![Value::Bool(v.is_nan())])
}

fn isinf(vm: &mut Vm, c: NativeCall) -> Ret {
    let [v] = nums(vm, &c)?;
    Ok(vec![Value::Bool(v.is_infinite())])
}

fn isfinite(vm: &mut Vm, c: NativeCall) -> Ret {
    let [v] = nums(vm, &c)?;
    Ok(vec![Value::Bool(v.is_finite())])
}

// ----------------------------------------------------------------- bit

/// `luaL_checkinteger` as 32 bits (unsigned).
fn unsigned(vm: &mut Vm, c: &NativeCall, n: usize) -> Result<u32, LuaError> {
    Ok(vm.check_int(c, n)? as u32)
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

// Shifts by 32 or more are undefined in C; ARM's give 0 (the probed
// client), and so do these.
fn lshift(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = unsigned(vm, &c, 1)?;
    let n = unsigned(vm, &c, 2)?;
    Ok(vec![Value::Num(f64::from(x.checked_shl(n).unwrap_or(0)))])
}

fn rshift(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = unsigned(vm, &c, 1)?;
    let n = unsigned(vm, &c, 2)?;
    Ok(vec![Value::Num(f64::from(x.checked_shr(n).unwrap_or(0)))])
}

fn arshift(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = vm.check_int(&c, 1)? as i32;
    let n = vm.check_int(&c, 2)? as i32;
    let r = if (0..32).contains(&n) {
        x >> n
    } else if x < 0 {
        -1
    } else {
        0
    };
    Ok(vec![Value::Num(f64::from(r))])
}

fn bit_mod(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = vm.check_int(&c, 1)? as i32;
    let y = vm.check_int(&c, 2)? as i32;
    // Division by zero traps on x86 and gives 0 on ARM.
    Ok(vec![Value::Num(f64::from(x.checked_rem(y).unwrap_or(0)))])
}

// ---------------------------------------------------------------- misc

fn tostringall(vm: &mut Vm, c: NativeCall) -> Ret {
    let mut out = Vec::with_capacity(c.len());
    for &v in &c.args {
        out.push(vm.tostring(v)?);
    }
    Ok(out)
}

fn nop(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// `assertf(value, format, ...)`: `assert` with a formatted message.
fn assertf(vm: &mut Vm, c: NativeCall) -> Ret {
    if c.arg(1).truthy() {
        return Ok(c.args);
    }
    if c.len() < 2 {
        return Err(vm.error("assertion failed!"));
    }
    let format = vm.get_global("string");
    let format = match format {
        Value::Table(t) => {
            let k = vm.str("format");
            vm.heap.table(t).get(k)
        }
        _ => Value::Nil,
    };
    let msg = vm.call(format, &c.args[1..])?;
    let msg = msg.first().copied().unwrap_or_default();
    Err(vm.raise(msg))
}

fn getglobal(vm: &mut Vm, c: NativeCall) -> Ret {
    let k = vm.check_str(&c, 1)?;
    let g = vm.globals;
    Ok(vec![vm.heap.table(g).get_str(k)])
}

fn setglobal(vm: &mut Vm, c: NativeCall) -> Ret {
    let k = vm.check_str(&c, 1)?;
    let g = vm.globals;
    vm.set_index(Value::Table(g), Value::Str(k), c.arg(2), None)?;
    Ok(vec![])
}

fn ipairs_reverse_step(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = vm.check_int(&c, 2)? - 1;
    if i < 1 {
        return Ok(vec![Value::Nil]);
    }
    let v = vm.index(c.arg(1), Value::Num(i as f64), None)?;
    Ok(vec![Value::Num(i as f64), v])
}

/// `ipairs_reverse(t)`: `#t` down to 1.
fn ipairs_reverse(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let n = vm.heap.table(t).border();
    let step = vm.native("ipairs_reverse_step", ipairs_reverse_step);
    Ok(vec![step, Value::Table(t), Value::Num((n + 1) as f64)])
}

/// `pcallwithenv(f, env, ...)`: `pcall` with `f`'s environment replaced
/// for the call (elune's `luaB_pcallwithenv`).
fn pcallwithenv(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Func(f), Value::Table(env)) = (c.arg(1), c.arg(2)) else {
        return Err(vm.error("Usage: local success, ... = pcallwithenv(f, env, [args...])"));
    };
    let old = vm.func_env(f);
    vm.set_func_env(f, env);
    let r = vm.pcall(Value::Func(f), c.args.get(2..).unwrap_or(&[]));
    vm.set_func_env(f, old);
    Ok(match r {
        Ok(mut results) => {
            results.insert(0, Value::Bool(true));
            results
        }
        Err(e) => vec![Value::Bool(false), e.value],
    })
}

fn geterrorhandler(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.error_handler])
}

fn seterrorhandler(vm: &mut Vm, c: NativeCall) -> Ret {
    let f = vm.check_func(&c, 1)?;
    vm.error_handler = f;
    Ok(vec![])
}

fn getprinthandler(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.print_handler])
}

fn setprinthandler(vm: &mut Vm, c: NativeCall) -> Ret {
    let f = vm.check_func(&c, 1)?;
    vm.print_handler = f;
    Ok(vec![])
}

/// `print` hands its arguments to the print handler.
fn print(vm: &mut Vm, c: NativeCall) -> Ret {
    let handler = vm.print_handler;
    vm.call(handler, &c.args)?;
    Ok(vec![])
}

fn fastrandom(vm: &mut Vm, c: NativeCall) -> Ret {
    crate::stdlib::math::random(vm, c)
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
        Some(&v) => return Ok(vec![v]),
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
                out.extend_from_slice(b"...\n");
                while exists(level + nbase) {
                    level += 1;
                }
            }
            firstpart = false;
            continue;
        }
        let info = &levels[cur as usize];
        out.extend_from_slice(&info.short_src);
        out.push(b':');
        if info.currentline > 0 {
            out.extend_from_slice(format!("{}:", info.currentline).as_bytes());
        }
        match &info.name {
            Some((kind, name)) if matches!(*kind, "global" | "local" | "field" | "method") => {
                out.extend_from_slice(format!(" in function `{name}'").as_bytes());
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

/// One variable of `debuglocals` (elune's `aux_dumpvalue`).
fn dump_value(vm: &mut Vm, name: &str, v: Value, recurse: u32, out: &mut Vec<u8>) {
    let indent = if recurse == 0 { " " } else { "" };
    out.extend_from_slice(indent.as_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(b" = ");
    match v {
        Value::Nil => out.extend_from_slice(b"nil\n"),
        Value::Bool(b) => out.extend_from_slice(if b { b"true\n" } else { b"false\n" }),
        Value::Num(n) => {
            out.extend_from_slice(crate::number::fmt_num(n).as_bytes());
            out.push(b'\n');
        }
        Value::Str(s) => {
            let b = vm.str_bytes(s);
            let b = &b[..b.iter().position(|&x| x == 0).unwrap_or(b.len())];
            out.push(b'"');
            out.extend_from_slice(b);
            out.extend_from_slice(b"\"\n");
        }
        Value::Table(t) => {
            out.extend_from_slice(b"<table> {\n");
            if recurse > 0 {
                let entries: Vec<(Value, Value)> = vm.heap.table(t).entries().collect();
                for (k, x) in entries {
                    let key = match k {
                        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
                        Value::Num(n) => crate::number::fmt_num(n),
                        _ => String::new(),
                    };
                    dump_value(vm, &key, x, recurse - 1, out);
                }
            }
            out.extend_from_slice(indent.as_bytes());
            out.extend_from_slice(b"}\n");
        }
        Value::Func(f) => {
            let (source, line) = match vm.heap.func(f) {
                crate::function::Function::Lua(l) => {
                    (l.proto.source.to_vec(), i64::from(l.proto.line_defined))
                }
                crate::function::Function::Native(_) => (b"=[C]".to_vec(), -1),
            };
            out.extend_from_slice(b"<function> defined ");
            out.extend_from_slice(&source);
            out.extend_from_slice(format!(":{line}\n").as_bytes());
        }
        Value::Userdata(_) => out.extend_from_slice(b"<userdata>\n"),
        other => out.extend_from_slice(format!("<{}>\n", other.type_name()).as_bytes()),
    }
}
