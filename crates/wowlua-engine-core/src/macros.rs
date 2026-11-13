//! Macros: the account's (indices 1 to 120) and the character's (121 to
//! 138), each a name, an icon and a body. A world profile's `macros`
//! (recorded by `/stdtest snapshot`) and `mock.setMacros` set them;
//! Blizzard's macro window creates, edits and deletes them
//! (`UPDATE_MACROS`). The fresh character has none. An edited macro's
//! action slots are told changed after `UPDATE_MACROS`, then the bars'
//! cooldowns (recorded once, scenario macro: the macro window saving the
//! account macro on slot 64 as its tab changed). A macro that casts
//! nothing is no enabled action (its cooldown's `isEnabled` false;
//! recorded once, an empty macro; unverified for one that only talks).
//!
//! Running one (decision T601): `C_Macro.RunMacroText` (a secure button's
//! `macrotext`) and `RunMacro` (its `macro`) run the lines through the
//! callback Blizzard's UI gives `C_Macro.SetMacroExecuteLineCallback`, as
//! the client does: each line as typed into chat, so `/click`, `/cast` and
//! the client's other secure commands run as Blizzard's code has them.
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

/// Whether the macro at `index` casts or uses something (a `/cast` or
/// `/use` line; see the module).
pub fn casts_something(vm: &Vm, index: f64) -> bool {
    let Some(at) = find(vm, Value::Num(index)) else {
        return false;
    };
    get(vm, at).body.lines().any(|l| {
        let l = l.trim_start().to_lowercase();
        [
            "/cast",
            "/use",
            "/castsequence",
            "/castrandom",
            "/userandom",
        ]
        .iter()
        .any(|c| l.starts_with(c))
    })
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
        // No such macro: nothing (the full probe).
        return Ok(vec![]);
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
    let index = index_of(character, i);
    let slots: Vec<i64> = engine(vm)
        .world
        .character
        .actions
        .iter()
        .filter(|(_, a)| a.kind == "macro" && matches!(a.id, Value::Num(n) if n == index))
        .map(|(s, _)| *s)
        .collect();
    if !slots.is_empty() {
        for s in slots {
            widgets::fire_event(vm, "ACTIONBAR_SLOT_CHANGED", &[Value::Num(s as f64)]);
        }
        widgets::fire_event(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    }
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

/// `C_Macro.SetMacroExecuteLineCallback(cb)`: the function that runs a
/// macro's line (Blizzard's MacroExecutionManager gives it at
/// `PLAYER_ENTERING_WORLD`: its private chat edit box's `SendText`).
fn set_execute_line_callback(vm: &mut Vm, c: NativeCall) -> Ret {
    let cb = c.arg(1);
    if matches!(cb, Value::Func(_)) {
        widgets::keep(vm, cb);
        engine_mut(vm).world.macro_line = Some(cb);
    }
    Ok(vec![])
}

/// Runs a macro's text line by line through the line callback, each as
/// typed into chat (decision T601): the client's secure commands
/// (`/click`, `/cast`, …), `SlashCmdList`'s, chat. Empty lines and `#`
/// lines (`#showtooltip`) run nothing (unverified); `StopMacro`
/// (`/stopmacro`) stops the rest. The running execution goes on into the
/// callback (only secure code gets here: the runners are protected).
/// Without Blizzard's UI there is no callback: nothing runs.
pub fn run_text(vm: &mut Vm, body: &str) {
    let Some(cb) = engine(vm).world.macro_line else {
        return;
    };
    let outer = std::mem::replace(&mut engine_mut(vm).world.macro_stopped, false);
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let s = vm.str(line);
        vm.call_from_engine_in(cb, &[s], false);
        if engine(vm).world.macro_stopped {
            break;
        }
    }
    engine_mut(vm).world.macro_stopped = outer;
}

/// Runs the macro at `index` (a click binding's, `ExecuteBinding`); none
/// there, nothing.
pub fn run_at(vm: &mut Vm, index: f64) {
    if let Some(at) = find(vm, Value::Num(index)) {
        let body = get(vm, at).body;
        run_text(vm, &body);
    }
}

/// `C_Macro.RunMacroText(text, button)`: the text run as a macro (a
/// secure button's `macrotext`). The click's button is what `[button]`
/// sees (`GetMouseButtonClicked`; unverified for a call outside a click).
fn run_macro_text(vm: &mut Vm, c: NativeCall) -> Ret {
    let body = text(vm, c.arg(1)).unwrap_or_default();
    run_text(vm, &body);
    Ok(vec![])
}

/// `RunMacro(index or name, button)`: the macro's body run (a secure
/// button's `macro`); no such macro, nothing.
fn run_macro(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(at) = find(vm, c.arg(1)) {
        let body = get(vm, at).body;
        run_text(vm, &body);
    }
    Ok(vec![])
}

/// `StopMacro()`: the running macro's other lines do not run.
fn stop_macro(vm: &mut Vm, _c: NativeCall) -> Ret {
    engine_mut(vm).world.macro_stopped = true;
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
        (
            "C_Macro.SetMacroExecuteLineCallback",
            set_execute_line_callback,
        ),
        ("C_Macro.RunMacroText", run_macro_text),
        ("RunMacro", run_macro),
        ("StopMacro", stop_macro),
    ]
}
