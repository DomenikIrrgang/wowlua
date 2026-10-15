//! Lua 5.1's string library (`lstrlib.c`), without `string.dump` (WoW has
//! none). Strings index this table through their metatable.

use crate::function::NativeCall;
use crate::number::fmt_g;
use crate::stdlib::pattern::{Capture, MatchState, SPECIALS};
use crate::stdlib::{Ret, register};
use crate::taint::Slot;
use crate::value::Value;
use crate::vm::Vm;

pub fn open(vm: &mut Vm) {
    let t = register(
        vm,
        "string",
        &[
            ("len", len),
            ("sub", sub),
            ("upper", upper),
            ("lower", lower),
            ("rep", rep),
            ("reverse", reverse),
            ("byte", byte),
            ("char", char),
            ("format", format),
            ("find", find),
            ("match", match_),
            ("gsub", gsub),
        ],
    );
    let gmatch_fn = vm.native("gmatch", gmatch);
    let k = vm.heap.intern(b"gmatch");
    vm.heap.table_mut(t).set_str(k, gmatch_fn);
    let meta = vm.new_table();
    let index = vm.names.index;
    vm.heap.table_mut(meta).set_str(index, Value::Table(t));
    vm.string_meta = Some(meta);
}

/// `posrelat`: a negative position counts from the end.
fn posrelat(pos: i64, len: usize) -> i64 {
    let pos = if pos < 0 { len as i64 + pos + 1 } else { pos };
    pos.max(0)
}

fn len(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    Ok(vec![Value::Num(s.len() as f64)])
}

fn sub(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let l = s.len();
    let mut start = posrelat(vm.check_int(&c, 2)?, l);
    let mut end = posrelat(vm.opt_int(&c, 3, -1)?, l);
    if start < 1 {
        start = 1;
    }
    if end > l as i64 {
        end = l as i64;
    }
    if start <= end {
        Ok(vec![vm.bytes(&s[start as usize - 1..end as usize])])
    } else {
        Ok(vec![vm.bytes(b"")])
    }
}

fn upper(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let u = if vm.rules.utf8_case {
        map_utf8(&s, |ch| ch.to_uppercase())
    } else {
        s.iter().map(u8::to_ascii_uppercase).collect()
    };
    Ok(vec![vm.bytes(&u)])
}

fn lower(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let u = if vm.rules.utf8_case {
        map_utf8(&s, |ch| ch.to_lowercase())
    } else {
        s.iter().map(u8::to_ascii_lowercase).collect()
    };
    Ok(vec![vm.bytes(&u)])
}

/// Case mapping of UTF-8 text (the client's `upper`/`lower`): characters
/// with a one-character mapping change, invalid bytes stay as they are.
fn map_utf8<I: Iterator<Item = char>>(s: &[u8], f: impl Fn(char) -> I) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for chunk in s.utf8_chunks() {
        for ch in chunk.valid().chars() {
            let mut mapped = f(ch);
            let c = match (mapped.next(), mapped.next()) {
                (Some(m), None) => m,
                _ => ch,
            };
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
        out.extend_from_slice(chunk.invalid());
    }
    out
}

fn rep(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let n = vm.check_int(&c, 2)?;
    if n <= 0 || s.is_empty() {
        return Ok(vec![vm.bytes(b"")]);
    }
    let total = s.len().saturating_mul(n as usize);
    if total > 1 << 31 {
        return Err(vm.error("not enough memory"));
    }
    Ok(vec![vm.bytes(&s.repeat(n as usize))])
}

fn reverse(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let r: Vec<u8> = s.iter().rev().copied().collect();
    Ok(vec![vm.bytes(&r)])
}

fn byte(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let l = s.len();
    let posi = posrelat(vm.opt_int(&c, 2, 1)?, l);
    let pose = posrelat(vm.opt_int(&c, 3, posi)?, l);
    let posi = posi.max(1);
    let pose = pose.min(l as i64);
    if posi > pose {
        return Ok(vec![]);
    }
    if pose - posi >= i32::MAX as i64 {
        return Err(vm.error("string slice too long"));
    }
    Ok(s[posi as usize - 1..pose as usize]
        .iter()
        .map(|&b| Value::Num(f64::from(b)))
        .collect())
}

fn char(vm: &mut Vm, c: NativeCall) -> Ret {
    let mut out = Vec::with_capacity(c.len());
    for i in 1..=c.len() {
        let v = vm.check_int(&c, i)?;
        if !(0..=255).contains(&v) {
            return Err(vm.arg_error(i, "invalid value"));
        }
        out.push(v as u8);
    }
    Ok(vec![vm.bytes(&out)])
}

// ---------------------------------------------------------------- format

const FLAGS: &[u8] = b"-+ #0";

#[derive(Default)]
struct Spec {
    minus: bool,
    plus: bool,
    space: bool,
    alt: bool,
    zero: bool,
    width: usize,
    precision: Option<usize>,
}

/// Pads `body` (with `prefix` such as a sign or `0x`) to the width.
fn pad(spec: &Spec, prefix: &[u8], body: &[u8], zero_ok: bool) -> Vec<u8> {
    let len = prefix.len() + body.len();
    let mut out = Vec::with_capacity(len.max(spec.width));
    if len >= spec.width {
        out.extend_from_slice(prefix);
        out.extend_from_slice(body);
    } else if spec.minus {
        out.extend_from_slice(prefix);
        out.extend_from_slice(body);
        out.resize(spec.width, b' ');
    } else if spec.zero && zero_ok {
        out.extend_from_slice(prefix);
        out.resize(spec.width - body.len(), b'0');
        out.extend_from_slice(body);
    } else {
        out.resize(spec.width - len, b' ');
        out.extend_from_slice(prefix);
        out.extend_from_slice(body);
    }
    out
}

fn sign_prefix(spec: &Spec, negative: bool) -> &'static [u8] {
    if negative {
        b"-"
    } else if spec.plus {
        b"+"
    } else if spec.space {
        b" "
    } else {
        b""
    }
}

fn fmt_int(spec: &Spec, v: i64) -> Vec<u8> {
    let mut digits = v.unsigned_abs().to_string().into_bytes();
    if let Some(p) = spec.precision {
        if p == 0 && v == 0 {
            digits.clear();
        }
        while digits.len() < p {
            digits.insert(0, b'0');
        }
    }
    pad(
        spec,
        sign_prefix(spec, v < 0),
        &digits,
        spec.precision.is_none(),
    )
}

fn fmt_unsigned(spec: &Spec, v: u64, conv: u8) -> Vec<u8> {
    let mut digits = match conv {
        b'o' => format!("{v:o}"),
        b'x' => format!("{v:x}"),
        b'X' => format!("{v:X}"),
        _ => v.to_string(),
    }
    .into_bytes();
    if let Some(p) = spec.precision {
        if p == 0 && v == 0 {
            digits.clear();
        }
        while digits.len() < p {
            digits.insert(0, b'0');
        }
    }
    let mut prefix: &[u8] = b"";
    if spec.alt {
        match conv {
            b'o' if digits.first() != Some(&b'0') => digits.insert(0, b'0'),
            b'x' if v != 0 => prefix = b"0x",
            b'X' if v != 0 => prefix = b"0X",
            _ => {}
        }
    }
    pad(spec, prefix, &digits, spec.precision.is_none())
}

fn fmt_float(spec: &Spec, v: f64, conv: u8) -> Vec<u8> {
    let upper = conv.is_ascii_uppercase();
    let negative = v.is_sign_negative() && !v.is_nan();
    let a = v.abs();
    if !v.is_finite() {
        let body = if v.is_nan() { "nan" } else { "inf" };
        let body = if upper {
            body.to_uppercase()
        } else {
            body.to_string()
        };
        return pad(spec, sign_prefix(spec, negative), body.as_bytes(), false);
    }
    let prec = spec.precision.unwrap_or(6);
    let body = match conv.to_ascii_lowercase() {
        b'f' => {
            let mut s = format!("{a:.prec$}");
            if spec.alt && prec == 0 {
                s.push('.');
            }
            s
        }
        b'e' => {
            let s = format!("{a:.prec$e}");
            let (mant, exp) = s.split_once('e').expect("exponent");
            let x: i32 = exp.parse().expect("digits");
            let mut m = mant.to_string();
            if spec.alt && prec == 0 {
                m.push('.');
            }
            let sign = if x < 0 { '-' } else { '+' };
            let e = if upper { 'E' } else { 'e' };
            format!("{m}{e}{sign}{:02}", x.abs())
        }
        _ => fmt_g(a, if prec == 0 { 1 } else { prec }, upper, spec.alt),
    };
    pad(spec, sign_prefix(spec, negative), body.as_bytes(), true)
}

fn fmt_str(spec: &Spec, s: &[u8]) -> Vec<u8> {
    // A C string ends at its first NUL.
    let s = match s.iter().position(|&b| b == 0) {
        Some(i) => &s[..i],
        None => s,
    };
    let s = match spec.precision {
        Some(p) if p < s.len() => &s[..p],
        _ => s,
    };
    pad(spec, b"", s, false)
}

fn addquoted(out: &mut Vec<u8>, s: &[u8]) {
    out.push(b'"');
    for &b in s {
        match b {
            b'"' | b'\\' | b'\n' => {
                out.push(b'\\');
                out.push(b);
            }
            b'\r' => out.extend_from_slice(b"\\r"),
            0 => out.extend_from_slice(b"\\000"),
            _ => out.push(b),
        }
    }
    out.push(b'"');
}

fn format(vm: &mut Vm, c: NativeCall) -> Ret {
    if vm.rules.client_format {
        return format_client(vm, c);
    }
    let fmt = vm.check_bytes(&c, 1)?;
    let top = c.len();
    let mut arg = 1;
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' {
            out.push(fmt[i]);
            i += 1;
            continue;
        }
        i += 1;
        if fmt.get(i) == Some(&b'%') {
            out.push(b'%');
            i += 1;
            continue;
        }
        arg += 1;
        if arg > top {
            return Err(vm.arg_error(arg, "no value"));
        }
        // scanformat
        let start = i;
        let mut spec = Spec::default();
        while i < fmt.len() && FLAGS.contains(&fmt[i]) {
            match fmt[i] {
                b'-' => spec.minus = true,
                b'+' => spec.plus = true,
                b' ' => spec.space = true,
                b'#' => spec.alt = true,
                _ => spec.zero = true,
            }
            i += 1;
        }
        // C: `>= sizeof(FLAGS)`, which counts the NUL.
        if i - start > FLAGS.len() {
            return Err(vm.error("invalid format (repeated flags)"));
        }
        let mut width = 0;
        for _ in 0..2 {
            if i < fmt.len() && fmt[i].is_ascii_digit() {
                width = width * 10 + (fmt[i] - b'0') as usize;
                i += 1;
            }
        }
        spec.width = width;
        if fmt.get(i) == Some(&b'.') {
            i += 1;
            let mut p = 0;
            for _ in 0..2 {
                if i < fmt.len() && fmt[i].is_ascii_digit() {
                    p = p * 10 + (fmt[i] - b'0') as usize;
                    i += 1;
                }
            }
            spec.precision = Some(p);
        }
        if i < fmt.len() && fmt[i].is_ascii_digit() {
            return Err(vm.error("invalid format (width or precision too long)"));
        }
        let conv = fmt.get(i).copied().unwrap_or(0);
        i += 1;
        let item = match conv {
            b'c' => {
                let n = vm.check_number(&c, arg)?;
                let ch = (n as i64) as u8;
                pad(&spec, b"", &[ch], false)
            }
            b'd' | b'i' => {
                let n = vm.check_number(&c, arg)?;
                fmt_int(&spec, n as i64)
            }
            b'o' | b'u' | b'x' | b'X' => {
                let n = vm.check_number(&c, arg)?;
                // C converts a negative double to unsigned differently per
                // CPU (undefined behavior); this follows ARM (and Rust):
                // saturation.
                let u = n as u64;
                fmt_unsigned(&spec, u, conv)
            }
            b'e' | b'E' | b'f' | b'g' | b'G' => {
                let n = vm.check_number(&c, arg)?;
                fmt_float(&spec, n, conv)
            }
            b'q' => {
                let s = vm.check_bytes(&c, arg)?;
                addquoted(&mut out, &s);
                continue;
            }
            b's' => {
                let s = vm.check_bytes(&c, arg)?;
                if spec.precision.is_none() && s.len() >= 100 {
                    out.extend_from_slice(&s);
                    continue;
                }
                fmt_str(&spec, &s)
            }
            _ => {
                let ch = conv as char;
                return Err(vm.error(&format!("invalid option '%{ch}' to 'format'")));
            }
        };
        // `strlen` of the formatted item.
        let n = item.iter().position(|&b| b == 0).unwrap_or(item.len());
        out.extend_from_slice(&item[..n]);
    }
    Ok(vec![vm.bytes(&out)])
}

/// The client's `string.format` (elune's `str_format`): positional
/// arguments (`%2$s`), `%F`, `%d` through the overflow-checked `lua_tolong`
/// (a value that is not a number counts as 0), no check for missing
/// arguments beyond the argument checks themselves.
fn format_client(vm: &mut Vm, c: NativeCall) -> Ret {
    const MAX_MATCH_LEN: usize = 20 - 4;
    let fmt = vm.check_bytes(&c, 1)?;
    let mut arg = 1;
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] != b'%' {
            out.push(fmt[i]);
            i += 1;
            continue;
        }
        i += 1;
        if fmt.get(i) == Some(&b'%') {
            out.push(b'%');
            i += 1;
            continue;
        }
        arg += 1;
        // scanarg: `%N$` (one or two digits) selects argument N.
        let mut p = i;
        let mut n: i64 = -1;
        for _ in 0..2 {
            if let Some(d) = fmt.get(p).filter(|d| d.is_ascii_digit()) {
                n = if n < 0 { 0 } else { n * 10 } + i64::from(d - b'0');
                p += 1;
            }
        }
        if fmt.get(p) == Some(&b'$') && n >= 0 {
            arg = n as usize + 1;
            i = p + 1;
        }
        // scanformat: `[-+ #0]*(%d*)%.?(%d*)`.
        let start = i;
        let mut spec = Spec::default();
        while i < fmt.len() && FLAGS.contains(&fmt[i]) {
            match fmt[i] {
                b'-' => spec.minus = true,
                b'+' => spec.plus = true,
                b' ' => spec.space = true,
                b'#' => spec.alt = true,
                _ => spec.zero = true,
            }
            i += 1;
        }
        let digits = |i: &mut usize| {
            let from = *i;
            let mut v = 0usize;
            while *i < fmt.len() && fmt[*i].is_ascii_digit() {
                v = v
                    .saturating_mul(10)
                    .saturating_add((fmt[*i] - b'0') as usize);
                *i += 1;
            }
            (v, *i - from)
        };
        let (width, wlen) = digits(&mut i);
        spec.width = width;
        let mut plen = 0;
        if fmt.get(i) == Some(&b'.') {
            i += 1;
            let (p, len) = digits(&mut i);
            plen = len;
            spec.precision = Some(p);
        }
        if wlen > 2 || plen > 2 || i - start > MAX_MATCH_LEN {
            return Err(vm.error("invalid format (width or precision too long)"));
        }
        let conv = fmt.get(i).copied().unwrap_or(0);
        i += 1;
        let item = match conv {
            b'c' => {
                let n = vm.check_number(&c, arg)?;
                pad(&spec, b"", &[(n as i64) as u8], false)
            }
            b'd' | b'i' => {
                let n = vm.tonumber(c.arg(arg)).unwrap_or(0.0);
                vm.check_overflow(n)?;
                fmt_int(&spec, n as i64)
            }
            b'o' | b'u' | b'x' | b'X' => {
                let n = vm.check_number(&c, arg)?;
                // `(unsigned long)` of a negative double: ARM saturates.
                fmt_unsigned(&spec, n as u64, conv)
            }
            b'e' | b'E' | b'f' | b'g' | b'G' | b'F' => {
                let n = vm.check_number(&c, arg)?;
                let conv = if conv == b'F' { b'f' } else { conv };
                fmt_float(&spec, n, conv)
            }
            b'q' => {
                let s = vm.check_bytes(&c, arg)?;
                addquoted(&mut out, &s);
                continue;
            }
            b's' => {
                let s = vm.check_bytes(&c, arg)?;
                if spec.precision.is_none() && s.len() >= 100 {
                    out.extend_from_slice(&s);
                    continue;
                }
                fmt_str(&spec, &s)
            }
            _ => return Err(vm.error("invalid option in `format'")),
        };
        let n = item.iter().position(|&b| b == 0).unwrap_or(item.len());
        out.extend_from_slice(&item[..n]);
    }
    Ok(vec![vm.bytes(&out)])
}

// -------------------------------------------------------------- patterns

fn capture_value(
    vm: &mut Vm,
    ms: &MatchState,
    i: usize,
    s: usize,
    e: usize,
) -> Result<Value, crate::error::LuaError> {
    if i >= ms.level {
        if i == 0 {
            return Ok(vm.bytes(&ms.src[s..e]));
        }
        return Err(vm.error("invalid capture index"));
    }
    match ms.capture[i] {
        Capture::Unfinished(_) => Err(vm.error("unfinished capture")),
        Capture::Position(p) => Ok(Value::Num((p + 1) as f64)),
        Capture::Range(a, b) => Ok(vm.bytes(&ms.src[a..b])),
    }
}

/// The captures (or the whole match when there are none).
fn push_captures(vm: &mut Vm, ms: &MatchState, s: usize, e: usize, whole: bool) -> Ret {
    let n = if ms.level == 0 && whole { 1 } else { ms.level };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push(capture_value(vm, ms, i, s, e)?);
    }
    Ok(out)
}

fn find_aux(vm: &mut Vm, c: NativeCall, find: bool) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let p = vm.check_bytes(&c, 2)?;
    let l1 = s.len();
    let mut init = posrelat(vm.opt_int(&c, 3, 1)?, l1) - 1;
    if init < 0 {
        init = 0;
    } else if init as usize > l1 {
        init = l1 as i64;
    }
    let init = init as usize;
    // `strpbrk` stops at a NUL in the pattern.
    let pcstr = match p.iter().position(|&b| b == 0) {
        Some(i) => &p[..i],
        None => &p[..],
    };
    let plain = c.arg(4).truthy() || !pcstr.iter().any(|b| SPECIALS.contains(b));
    if find && plain {
        if let Some(pos) = memfind(&s[init..], &p) {
            let start = init + pos;
            return Ok(vec![
                Value::Num((start + 1) as f64),
                Value::Num((start + p.len()) as f64),
            ]);
        }
        return Ok(vec![Value::Nil]);
    }
    let (anchor, pat) = match p.first() {
        Some(b'^') => (true, &p[1..]),
        _ => (false, &p[..]),
    };
    let mut s1 = init;
    loop {
        let mut ms = MatchState::new(&s, pat);
        match ms.do_match(s1, 0) {
            Err(msg) => return Err(vm.error(&msg)),
            Ok(Some(res)) => {
                if find {
                    let mut out = vec![Value::Num((s1 + 1) as f64), Value::Num(res as f64)];
                    out.extend(push_captures(vm, &ms, 0, 0, false)?);
                    return Ok(out);
                }
                return push_captures(vm, &ms, s1, res, true);
            }
            Ok(None) => {}
        }
        s1 += 1;
        if s1 > l1 || anchor {
            break;
        }
    }
    Ok(vec![Value::Nil])
}

fn memfind(s: &[u8], p: &[u8]) -> Option<usize> {
    if p.is_empty() {
        return Some(0);
    }
    if p.len() > s.len() {
        return None;
    }
    s.windows(p.len()).position(|w| w == p)
}

fn find(vm: &mut Vm, c: NativeCall) -> Ret {
    find_aux(vm, c, true)
}

fn match_(vm: &mut Vm, c: NativeCall) -> Ret {
    find_aux(vm, c, false)
}

fn gmatch(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_str(&c, 1)?;
    let p = vm.check_str(&c, 2)?;
    // The iterator's state: string, pattern and position, in a table.
    let state = vm.new_table();
    let t = vm.heap.table_mut(state);
    t.set_int(1, Slot::secure(Value::Str(s)));
    t.set_int(2, Slot::secure(Value::Str(p)));
    t.set_int(3, Slot::secure(Value::Num(0.0)));
    let iter = vm.native_with("gmatch_iterator", gmatch_step, vec![Value::Table(state)]);
    Ok(vec![iter])
}

fn gmatch_step(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(state) = c.upvals[0] else {
        unreachable!()
    };
    let t = vm.heap.table(state);
    let (Value::Str(s), Value::Str(p), Value::Num(pos)) =
        (t.get_int(1), t.get_int(2), t.get_int(3))
    else {
        unreachable!()
    };
    let src = vm.heap.str_rc(s);
    let pat = vm.heap.str_rc(p);
    let mut start = pos as usize;
    while start <= src.len() {
        let mut ms = MatchState::new(&src, &pat);
        match ms.do_match(start, 0) {
            Err(msg) => return Err(vm.error(&msg)),
            Ok(Some(e)) => {
                let newstart = if e == start { e + 1 } else { e };
                vm.heap
                    .table_mut(state)
                    .set_int(3, Slot::secure(Value::Num(newstart as f64)));
                return push_captures(vm, &ms, start, e, true);
            }
            Ok(None) => start += 1,
        }
    }
    Ok(vec![Value::Nil])
}

fn gsub(vm: &mut Vm, c: NativeCall) -> Ret {
    let src = vm.check_bytes(&c, 1)?;
    let p = vm.check_bytes(&c, 2)?;
    let repl = c.arg(3);
    let max_s = vm.opt_int(&c, 4, src.len() as i64 + 1)?;
    if !matches!(
        repl,
        Value::Num(_) | Value::Str(_) | Value::Table(_) | Value::Func(_)
    ) {
        return Err(vm.arg_error(3, "string/function/table expected"));
    }
    let (anchor, pat) = match p.first() {
        Some(b'^') => (true, &p[1..]),
        _ => (false, &p[..]),
    };
    let repl_bytes = match repl {
        Value::Str(_) | Value::Num(_) => {
            let mut b = Vec::new();
            vm.append_strlike(&mut b, repl);
            Some(b)
        }
        _ => None,
    };
    let mut out = Vec::new();
    let mut s = 0;
    let mut n: i64 = 0;
    while n < max_s {
        let mut ms = MatchState::new(&src, pat);
        let e = match ms.do_match(s, 0) {
            Err(msg) => return Err(vm.error(&msg)),
            Ok(e) => e,
        };
        if let Some(e) = e {
            n += 1;
            // add_value
            let value = match &repl_bytes {
                Some(r) => {
                    add_s(vm, &ms, &mut out, r, s, e)?;
                    None
                }
                None => match repl {
                    Value::Func(_) => {
                        let caps = push_captures(vm, &ms, s, e, true)?;
                        let r = vm.call(repl, &caps)?;
                        Some(r.first().copied().unwrap_or_default())
                    }
                    _ => {
                        let key = capture_value(vm, &ms, 0, s, e)?;
                        Some(vm.index(repl, key, None)?)
                    }
                },
            };
            if let Some(v) = value {
                if !v.truthy() {
                    out.extend_from_slice(&src[s..e]);
                } else if matches!(v, Value::Str(_) | Value::Num(_)) {
                    vm.append_strlike(&mut out, v);
                } else {
                    return Err(
                        vm.error(&format!("invalid replacement value (a {})", v.type_name()))
                    );
                }
            }
        }
        match e {
            Some(e) if e > s => s = e,
            _ if s < src.len() => {
                out.push(src[s]);
                s += 1;
            }
            _ => break,
        }
        if anchor {
            break;
        }
    }
    out.extend_from_slice(&src[s.min(src.len())..]);
    let r = vm.bytes(&out);
    Ok(vec![r, Value::Num(n as f64)])
}

/// A replacement string with `%0`–`%9` (`add_s`).
fn add_s(
    vm: &mut Vm,
    ms: &MatchState,
    out: &mut Vec<u8>,
    news: &[u8],
    s: usize,
    e: usize,
) -> Result<(), crate::error::LuaError> {
    let mut i = 0;
    while i < news.len() {
        if news[i] != b'%' {
            out.push(news[i]);
            i += 1;
            continue;
        }
        i += 1;
        // A trailing `%` reads the string's terminating NUL in C.
        let ch = news.get(i).copied().unwrap_or(0);
        if !ch.is_ascii_digit() {
            out.push(ch);
        } else if ch == b'0' {
            out.extend_from_slice(&ms.src[s..e]);
        } else {
            let v = capture_value(vm, ms, (ch - b'1') as usize, s, e)?;
            vm.append_strlike(out, v);
        }
        i += 1;
    }
    Ok(())
}
