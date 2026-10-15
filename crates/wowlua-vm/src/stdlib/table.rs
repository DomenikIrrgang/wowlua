//! Lua 5.1's table library (`ltablib.c`). `sort` is Lua's own quicksort, so
//! comparators see the same calls in the same order as in the client.

use crate::error::LuaError;
use crate::function::NativeCall;
use crate::heap::TableRef;
use crate::stdlib::{Ret, register};
use crate::value::Value;
use crate::vm::Vm;

pub fn open(vm: &mut Vm) {
    register(
        vm,
        "table",
        &[
            ("concat", concat),
            ("foreach", foreach),
            ("foreachi", foreachi),
            ("getn", getn),
            ("maxn", maxn),
            ("insert", insert),
            ("remove", remove),
            ("setn", setn),
            ("sort", sort),
        ],
    );
}

fn aux_getn(vm: &mut Vm, c: &NativeCall, n: usize) -> Result<(TableRef, usize), LuaError> {
    let t = vm.check_table(c, n)?;
    Ok((t, vm.heap.table(t).border()))
}

fn rawgeti(vm: &Vm, t: TableRef, i: i64) -> Value {
    vm.heap.table(t).get_num(i as f64)
}

fn rawseti(vm: &mut Vm, t: TableRef, i: i64, v: Value) {
    let _ = vm.heap.table_mut(t).set(Value::Num(i as f64), v);
}

fn foreachi(vm: &mut Vm, c: NativeCall) -> Ret {
    let (t, n) = aux_getn(vm, &c, 1)?;
    let f = vm.check_func(&c, 2)?;
    for i in 1..=n as i64 {
        let v = rawgeti(vm, t, i);
        let r = vm.call(f, &[Value::Num(i as f64), v])?;
        let first = r.first().copied().unwrap_or_default();
        if !first.is_nil() {
            return Ok(vec![first]);
        }
    }
    Ok(vec![])
}

fn foreach(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let f = vm.check_func(&c, 2)?;
    let mut key = Value::Nil;
    loop {
        let next = match vm.heap.table(t).next(key) {
            Ok(n) => n,
            Err(_) => return Err(vm.rt_error("invalid key to 'next'")),
        };
        let Some((k, v)) = next else { break };
        let r = vm.call(f, &[k, v])?;
        let first = r.first().copied().unwrap_or_default();
        if !first.is_nil() {
            return Ok(vec![first]);
        }
        key = k;
    }
    Ok(vec![])
}

fn maxn(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let mut max = 0.0f64;
    for (k, _) in vm.heap.table(t).entries() {
        if let Value::Num(n) = k
            && n > max
        {
            max = n;
        }
    }
    Ok(vec![Value::Num(max)])
}

fn getn(vm: &mut Vm, c: NativeCall) -> Ret {
    let (_, n) = aux_getn(vm, &c, 1)?;
    Ok(vec![Value::Num(n as f64)])
}

fn setn(vm: &mut Vm, c: NativeCall) -> Ret {
    vm.check_table(&c, 1)?;
    Err(vm.error("'setn' is obsolete"))
}

fn insert(vm: &mut Vm, c: NativeCall) -> Ret {
    let (t, n) = aux_getn(vm, &c, 1)?;
    let mut e = n as i64 + 1;
    let pos = match c.len() {
        2 => e,
        3 => {
            let pos = vm.check_int(&c, 2)?;
            if pos > e {
                e = pos;
            }
            let mut i = e;
            while i > pos {
                let v = rawgeti(vm, t, i - 1);
                rawseti(vm, t, i, v);
                i -= 1;
            }
            pos
        }
        _ => return Err(vm.error("wrong number of arguments to 'insert'")),
    };
    let v = c.arg(c.len());
    rawseti(vm, t, pos, v);
    Ok(vec![])
}

fn remove(vm: &mut Vm, c: NativeCall) -> Ret {
    let (t, n) = aux_getn(vm, &c, 1)?;
    let e = n as i64;
    let mut pos = vm.opt_int(&c, 2, e)?;
    if !(1 <= pos && pos <= e) {
        return Ok(vec![]);
    }
    let result = rawgeti(vm, t, pos);
    while pos < e {
        let v = rawgeti(vm, t, pos + 1);
        rawseti(vm, t, pos, v);
        pos += 1;
    }
    rawseti(vm, t, e, Value::Nil);
    Ok(vec![result])
}

fn concat(vm: &mut Vm, c: NativeCall) -> Ret {
    let sep = match vm.opt_str(&c, 2)? {
        Some(s) => vm.heap.str_bytes(s).to_vec(),
        None => Vec::new(),
    };
    let t = vm.check_table(&c, 1)?;
    let mut i = vm.opt_int(&c, 3, 1)?;
    let last = if c.arg(4).is_nil() {
        vm.heap.table(t).border() as i64
    } else {
        vm.check_int(&c, 4)?
    };
    let mut out = Vec::new();
    let addfield = |vm: &mut Vm, out: &mut Vec<u8>, i: i64| -> Result<(), LuaError> {
        let v = rawgeti(vm, t, i);
        if !matches!(v, Value::Str(_) | Value::Num(_)) {
            let t = v.type_name();
            return Err(vm.error(&format!(
                "invalid value ({t}) at index {i} in table for 'concat'"
            )));
        }
        vm.append_strlike(out, v);
        Ok(())
    };
    while i < last {
        addfield(vm, &mut out, i)?;
        out.extend_from_slice(&sep);
        i += 1;
    }
    if i == last {
        addfield(vm, &mut out, i)?;
    }
    Ok(vec![vm.bytes(&out)])
}

/// The comparison `sort` uses: the function, or `<`.
fn sort_comp(vm: &mut Vm, f: Value, a: Value, b: Value) -> Result<bool, LuaError> {
    if f.is_nil() {
        vm.less_than(a, b)
    } else {
        let r = vm.call(f, &[a, b])?;
        Ok(r.first().copied().unwrap_or_default().truthy())
    }
}

fn sort(vm: &mut Vm, c: NativeCall) -> Ret {
    let (t, n) = aux_getn(vm, &c, 1)?;
    let f = if c.has(2) {
        vm.check_func(&c, 2)?
    } else {
        Value::Nil
    };
    auxsort(vm, t, f, 1, n as i64)?;
    Ok(vec![])
}

/// Lua 5.1's `auxsort`, step for step.
fn auxsort(vm: &mut Vm, t: TableRef, f: Value, mut l: i64, mut u: i64) -> Result<(), LuaError> {
    while l < u {
        // Sort a[l], a[(l+u)/2] and a[u].
        let al = rawgeti(vm, t, l);
        let au = rawgeti(vm, t, u);
        if sort_comp(vm, f, au, al)? {
            rawseti(vm, t, l, au);
            rawseti(vm, t, u, al);
        }
        if u - l == 1 {
            break;
        }
        let mut i = (l + u) / 2;
        let ai = rawgeti(vm, t, i);
        let al = rawgeti(vm, t, l);
        if sort_comp(vm, f, ai, al)? {
            rawseti(vm, t, i, al);
            rawseti(vm, t, l, ai);
        } else {
            let au = rawgeti(vm, t, u);
            if sort_comp(vm, f, au, ai)? {
                rawseti(vm, t, i, au);
                rawseti(vm, t, u, ai);
            }
        }
        if u - l == 2 {
            break;
        }
        let p = rawgeti(vm, t, i);
        let au1 = rawgeti(vm, t, u - 1);
        rawseti(vm, t, i, au1);
        rawseti(vm, t, u - 1, p);
        // a[l] <= P == a[u-1] <= a[u]: sort from l+1 to u-2.
        i = l;
        let mut j = u - 1;
        loop {
            let mut ai;
            loop {
                i += 1;
                ai = rawgeti(vm, t, i);
                if !sort_comp(vm, f, ai, p)? {
                    break;
                }
                if i > u {
                    return Err(vm.error("invalid order function for sorting"));
                }
            }
            let mut aj;
            loop {
                j -= 1;
                aj = rawgeti(vm, t, j);
                if !sort_comp(vm, f, p, aj)? {
                    break;
                }
                if j < l {
                    return Err(vm.error("invalid order function for sorting"));
                }
            }
            if j < i {
                break;
            }
            rawseti(vm, t, i, aj);
            rawseti(vm, t, j, ai);
        }
        let au1 = rawgeti(vm, t, u - 1);
        let ai = rawgeti(vm, t, i);
        rawseti(vm, t, u - 1, ai);
        rawseti(vm, t, i, au1);
        // Recurse into the smaller half, loop on the larger.
        let (jj, ii);
        if i - l < u - i {
            jj = l;
            ii = i - 1;
            l = i + 1;
        } else {
            jj = i + 1;
            ii = u;
            u = i - 1;
        }
        auxsort(vm, t, f, jj, ii)?;
    }
    Ok(())
}
