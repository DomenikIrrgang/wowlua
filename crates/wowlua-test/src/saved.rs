//! SavedVariables across `mock.reloadUI()`: the TOC's variables taken from
//! one VM and given to the next as the client writes and reads them back
//! (`/stdtest sv`, WoW Forever 70425):
//!
//! - strings, booleans and tables are kept; numbers to 16 significant
//!   digits (`-0` as `0`);
//! - functions, coroutines, userdata (a frame's `[0]`), secrets, NaN and
//!   infinities are left out, and so are entries with table keys;
//! - a table reached twice is written twice: two tables after the reload.
//!
//! Unverified: a table inside itself (left out here).

use wowlua_vm::{TableRef, Value, Vm};

/// A saved value.
#[derive(Clone, Debug, PartialEq)]
pub enum Saved {
    Bool(bool),
    Num(f64),
    Str(Vec<u8>),
    Table(Vec<(Saved, Saved)>),
}

/// The variables a TOC saves (`SavedVariables`, `SavedVariablesPerCharacter`,
/// each directive possibly repeated).
pub fn names(toc: &wowlua_toc::ParsedToc) -> Vec<String> {
    toc.directives
        .iter()
        .filter(|d| {
            d.key.eq_ignore_ascii_case("SavedVariables")
                || d.key.eq_ignore_ascii_case("SavedVariablesPerCharacter")
        })
        .flat_map(|d| d.value.split(',').map(|n| n.trim().to_string()))
        .filter(|n| !n.is_empty())
        .collect()
}

/// A number as the client writes it: 16 significant digits.
fn written(n: f64) -> Option<f64> {
    if !n.is_finite() {
        return None;
    }
    if n == 0.0 {
        return Some(0.0);
    }
    format!("{n:.15e}").parse().ok()
}

fn capture_value(vm: &Vm, v: Value, path: &mut Vec<TableRef>) -> Option<Saved> {
    match v {
        Value::Bool(b) => Some(Saved::Bool(b)),
        Value::Num(n) => written(n).map(Saved::Num),
        Value::Str(s) => Some(Saved::Str(vm.str_bytes(s).to_vec())),
        Value::Table(t) => {
            if path.contains(&t) {
                return None;
            }
            path.push(t);
            let mut entries = Vec::new();
            for (k, v) in vm.heap.table(t).entries() {
                let key = match k {
                    Value::Bool(_) | Value::Num(_) | Value::Str(_) => capture_value(vm, k, path),
                    _ => None,
                };
                if let (Some(key), Some(value)) = (key, capture_value(vm, v, path)) {
                    entries.push((key, value));
                }
            }
            path.pop();
            Some(Saved::Table(entries))
        }
        _ => None,
    }
}

/// The variables' values in `vm` (unset ones left out).
pub fn capture(vm: &mut Vm, names: &[String]) -> Vec<(String, Saved)> {
    names
        .iter()
        .filter_map(|name| {
            let v = vm.get_global(name);
            capture_value(vm, v, &mut Vec::new()).map(|s| (name.clone(), s))
        })
        .collect()
}

fn restore_value(vm: &mut Vm, s: &Saved) -> Value {
    match s {
        Saved::Bool(b) => Value::Bool(*b),
        Saved::Num(n) => Value::Num(*n),
        Saved::Str(bytes) => Value::Str(vm.heap.intern(bytes)),
        Saved::Table(entries) => {
            let t = vm.new_table();
            for (k, v) in entries {
                let k = restore_value(vm, k);
                let v = restore_value(vm, v);
                let _ = vm.heap.table_mut(t).set(k, v);
            }
            Value::Table(t)
        }
    }
}

/// Sets the saved variables in `vm`, as the client does before the addon's
/// `ADDON_LOADED`.
pub fn restore(vm: &mut Vm, saved: &[(String, Saved)]) {
    for (name, s) in saved {
        let v = restore_value(vm, s);
        vm.set_global(name, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_as_the_client_writes_them() {
        assert_eq!(written(0.12345678901234567), Some(0.1234567890123457));
        assert_eq!(written(1.0 / 3.0), Some(1.0 / 3.0));
        assert_eq!(written(0.1), Some(0.1));
        assert_eq!(written(2f64.powi(53) + 2.0), Some(9007199254740994.0));
        assert_eq!(written(-0.0).map(f64::is_sign_negative), Some(false));
        assert_eq!(written(f64::NAN), None);
        assert_eq!(written(f64::INFINITY), None);
    }
}
