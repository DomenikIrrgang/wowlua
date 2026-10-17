//! Macros: the account's (indices 1 to 120) and the character's (121 to
//! 138), each a name, an icon and a body. A world profile's `macros`
//! (recorded by `/stdtest snapshot`) and `mock.setMacros` set them;
//! Blizzard's macro window creates, edits and deletes them
//! (`UPDATE_MACROS`). The fresh character has none.
//!
//! Unverified: the order a new or renamed macro takes (kept by name, as
//! the client lists them), `GetMacroInfo`'s answer beyond name, icon and
//! body.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, widgets};

/// The account's macros come first, then the character's from here.
pub const CHARACTER_BASE: usize = 120;
pub const MAX_CHARACTER: usize = 18;

#[derive(Clone, Debug, Default)]
pub struct Macro {
    pub name: String,
    /// A file ID or a texture path.
    pub icon: Value,
    pub body: String,
}

#[derive(Clone, Debug, Default)]
pub struct Macros {
    pub account: Vec<Macro>,
    pub character: Vec<Macro>,
}

fn text(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

/// The macro an index or a name means: (character's, position).
fn find(vm: &Vm, v: Value) -> Option<(bool, usize)> {
    let m = &engine(vm).world.character.macros;
    match vm.hidden(v) {
        Value::Num(n) if n >= 1.0 => {
            let i = n as usize;
            if i <= CHARACTER_BASE {
                (i <= m.account.len()).then_some((false, i - 1))
            } else {
                let j = i - CHARACTER_BASE;
                (j <= m.character.len()).then_some((true, j - 1))
            }
        }
        Value::Str(s) => {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            m.account
                .iter()
                .position(|x| x.name == name)
                .map(|i| (false, i))
                .or_else(|| {
                    m.character
                        .iter()
                        .position(|x| x.name == name)
                        .map(|i| (true, i))
                })
        }
        _ => None,
    }
}

fn index_of(character: bool, i: usize) -> f64 {
    (if character { CHARACTER_BASE } else { 0 } + i + 1) as f64
}

fn get(vm: &Vm, (character, i): (bool, usize)) -> Macro {
    let m = &engine(vm).world.character.macros;
    if character {
        m.character[i].clone()
    } else {
        m.account[i].clone()
    }
}

fn get_num_macros(vm: &mut Vm, _c: NativeCall) -> Ret {
    let m = &engine(vm).world.character.macros;
    Ok(vec![
        Value::Num(m.account.len() as f64),
        Value::Num(m.character.len() as f64),
    ])
}

/// `GetMacroInfo(index or name)`: name, icon, body.
fn get_macro_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(at) = find(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let m = get(vm, at);
    Ok(vec![vm.str(&m.name), m.icon, vm.str(&m.body)])
}

fn get_macro_body(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(at) = find(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let m = get(vm, at);
    Ok(vec![vm.str(&m.body)])
}

/// `GetMacroIndexByName(name)`: its index, 0 for none.
fn get_macro_index_by_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let index = match c.arg(1) {
        Value::Str(_) => find(vm, c.arg(1)).map_or(0.0, |(ch, i)| index_of(ch, i)),
        _ => 0.0,
    };
    Ok(vec![Value::Num(index)])
}

/// Keeps a list by name; the new index of `name`.
fn place(list: &mut Vec<Macro>, m: Macro) -> usize {
    let name = m.name.clone();
    list.push(m);
    list.sort_by_key(|m| m.name.to_lowercase());
    list.iter().position(|x| x.name == name).unwrap_or(0)
}

/// `CreateMacro(name, icon, body, perCharacter)`: the new macro's index.
fn create_macro(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = text(vm, c.arg(1)).unwrap_or_default();
    let body = text(vm, c.arg(3)).unwrap_or_default();
    let icon = c.arg(2);
    widgets::keep(vm, icon);
    let character = c.arg(4).truthy();
    let full = {
        let m = &engine(vm).world.character.macros;
        if character {
            m.character.len() >= MAX_CHARACTER
        } else {
            m.account.len() >= CHARACTER_BASE
        }
    };
    if full {
        return Err(vm.error("Couldn't create macro: too many macros"));
    }
    let m = Macro { name, icon, body };
    let list = &mut engine_mut(vm).world.character.macros;
    let i = if character {
        place(&mut list.character, m)
    } else {
        place(&mut list.account, m)
    };
    widgets::fire_event(vm, "UPDATE_MACROS", &[]);
    Ok(vec![Value::Num(index_of(character, i))])
}

/// `EditMacro(index, name, icon, body)`: the given parts changed (nil
/// keeps one); the macro's index.
fn edit_macro(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((character, i)) = find(vm, c.arg(1)) else {
        return Ok(vec![Value::Num(0.0)]);
    };
    let mut m = get(vm, (character, i));
    if let Some(name) = text(vm, c.arg(2)) {
        m.name = name;
    }
    if !c.arg(3).is_nil() {
        m.icon = c.arg(3);
        widgets::keep(vm, m.icon);
    }
    if let Some(body) = text(vm, c.arg(4)) {
        m.body = body;
    }
    let list = &mut engine_mut(vm).world.character.macros;
    let target = if character {
        &mut list.character
    } else {
        &mut list.account
    };
    target.remove(i);
    let j = place(target, m);
    widgets::fire_event(vm, "UPDATE_MACROS", &[]);
    Ok(vec![Value::Num(index_of(character, j))])
}

fn delete_macro(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some((character, i)) = find(vm, c.arg(1)) {
        let list = &mut engine_mut(vm).world.character.macros;
        if character {
            list.character.remove(i);
        } else {
            list.account.remove(i);
        }
        widgets::fire_event(vm, "UPDATE_MACROS", &[]);
    }
    Ok(vec![])
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn macros_of(vm: &mut Vm, v: Value) -> Vec<Macro> {
    let Value::Table(list) = v else {
        return Vec::new();
    };
    let n = vm.heap.table(list).border();
    let mut out = Vec::new();
    for i in 1..=n {
        let Value::Table(t) = vm.heap.table(list).get(Value::Num(i as f64)) else {
            continue;
        };
        let name = field(vm, t, "name");
        let body = field(vm, t, "body");
        let icon = field(vm, t, "icon");
        widgets::keep(vm, icon);
        out.push(Macro {
            name: text(vm, name).unwrap_or_default(),
            icon,
            body: text(vm, body).unwrap_or_default(),
        });
    }
    out
}

/// `mock.setMacros({ account = { { name, icon, body }, … }, character =
/// { … } })`: the macros, in the client's order (`UPDATE_MACROS`).
pub fn control_set_macros(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("setMacros: a table expected"));
    };
    let account = field(vm, t, "account");
    let character = field(vm, t, "character");
    let account = macros_of(vm, account);
    let character = macros_of(vm, character);
    engine_mut(vm).world.character.macros = Macros { account, character };
    widgets::fire_event(vm, "UPDATE_MACROS", &[]);
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("GetNumMacros", get_num_macros),
        ("GetMacroInfo", get_macro_info),
        ("GetMacroBody", get_macro_body),
        ("GetMacroIndexByName", get_macro_index_by_name),
        ("CreateMacro", create_macro),
        ("EditMacro", edit_macro),
        ("DeleteMacro", delete_macro),
    ]
}
