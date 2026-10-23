//! Values as the debugger shows them (spec debugger §18.7): their text,
//! type, taint and secrecy, and what they expand to.

use wowlua_vm::heap::FuncRef;
use wowlua_vm::taint::Taint;
use wowlua_vm::{Slot, TableRef, Value, Vm};

use crate::controller::Paused;
use crate::protocol::Variable;

/// What a variables reference expands.
#[derive(Clone, Debug)]
pub(crate) enum RefTarget {
    Locals(usize),
    Varargs(usize),
    Upvalues(FuncRef),
    Table(TableRef),
    /// A widget's own view: type, name, state, layout; then its fields.
    Widget(TableRef),
}

/// Most entries a table shows.
const MAX_ENTRIES: usize = 1000;

pub(crate) fn add_ref(p: &mut Paused, t: RefTarget) -> u32 {
    p.refs.push(t);
    p.refs.len() as u32
}

/// Keeps `v` alive while stopped.
pub(crate) fn pin(vm: &mut Vm, p: &mut Paused, v: Value) {
    let n = vm.heap.table(p.pins).border();
    let _ = vm.heap.table_mut(p.pins).set(Value::Num((n + 1) as f64), v);
}

pub(crate) fn taint_text(vm: &Vm, t: Taint) -> String {
    if t.is_secure() {
        "secure".into()
    } else {
        vm.taint_name(t).unwrap_or_else(|| "tainted".into())
    }
}

fn str_text(vm: &Vm, v: Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

/// An error value's message.
pub(crate) fn error_text(vm: &mut Vm, v: Value) -> String {
    let v = vm.hidden(v);
    match str_text(vm, v) {
        Some(s) => s,
        None => String::from_utf8_lossy(&vm.raw_tostring(v)).into_owned(),
    }
}

/// A Lua string literal of `s`, cut after 4000 characters.
fn quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for (i, c) in s.chars().enumerate() {
        if i >= 4000 {
            out.push('…');
            break;
        }
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\{}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A value's text and type (a secret's: what it hides).
pub(crate) fn describe(vm: &mut Vm, v: Value) -> (String, String) {
    let v = vm.hidden(v);
    match v {
        Value::Nil => ("nil".into(), "nil".into()),
        Value::Bool(b) => (b.to_string(), "boolean".into()),
        Value::Num(n) => (wowlua_vm::number::fmt_num(n), "number".into()),
        Value::Str(_) => (
            quoted(&str_text(vm, v).unwrap_or_default()),
            "string".into(),
        ),
        Value::Table(_) => {
            if let Some(id) = wowlua_engine::widgets::widget_of(vm, v) {
                return (widget_summary(vm, id), "widget".into());
            }
            (
                String::from_utf8_lossy(&vm.raw_tostring(v)).into_owned(),
                "table".into(),
            )
        }
        Value::Func(f) => {
            let mut text = String::from_utf8_lossy(&vm.raw_tostring(v)).into_owned();
            match vm.func_proto(f) {
                Some(p) => {
                    let src =
                        String::from_utf8_lossy(&wowlua_vm::vm::chunkid(&p.source)).into_owned();
                    text.push_str(&format!(" ({src}:{})", p.line_defined));
                }
                None => text.push_str(&format!(" ([C] {})", vm.native_name(f))),
            }
            (text, "function".into())
        }
        other => (
            String::from_utf8_lossy(&vm.raw_tostring(other)).into_owned(),
            type_name(other).into(),
        ),
    }
}

fn type_name(v: Value) -> &'static str {
    match v {
        Value::Nil => "nil",
        Value::Bool(_) => "boolean",
        Value::Num(_) => "number",
        Value::Str(_) => "string",
        Value::Table(_) => "table",
        Value::Func(_) => "function",
        Value::Thread(_) => "thread",
        _ => "userdata",
    }
}

/// `Button "MyButton" shown 120x22`.
fn widget_summary(vm: &Vm, id: wowlua_engine::widgets::WidgetId) -> String {
    let e = wowlua_engine::engine(vm);
    let w = &e.world.widgets[id.0];
    let name = match &w.name {
        Some(n) => format!(" \"{n}\""),
        None => String::new(),
    };
    let state = if wowlua_engine::widgets::visible(e, id) {
        "visible"
    } else if w.shown {
        "shown"
    } else {
        "hidden"
    };
    let size = match wowlua_engine::widgets::bounds(vm, id, 0) {
        Some((_, _, w, h)) => format!(" {}x{}", fmt(w), fmt(h)),
        None => String::new(),
    };
    format!("{}{name} {state}{size}", w.class)
}

fn fmt(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{n:.0}")
    } else {
        format!("{n:.2}")
    }
}

/// A variable for `v` (its children through a new reference).
pub(crate) fn variable(vm: &mut Vm, p: &mut Paused, name: String, s: Slot) -> Variable {
    let secret = matches!(s.v, Value::Secret(_));
    let inner = vm.hidden(s.v);
    let (value, ty) = describe(vm, s.v);
    let reference = match inner {
        Value::Table(t) => {
            pin(vm, p, inner);
            if wowlua_engine::widgets::widget_of(vm, inner).is_some() {
                add_ref(p, RefTarget::Widget(t))
            } else {
                add_ref(p, RefTarget::Table(t))
            }
        }
        Value::Func(f) if !vm.upvalues(f).is_empty() => {
            pin(vm, p, inner);
            add_ref(p, RefTarget::Upvalues(f))
        }
        _ => 0,
    };
    Variable {
        name,
        value,
        ty,
        reference,
        taint: taint_text(vm, s.t),
        secret,
    }
}

/// The variables a reference expands to.
pub(crate) fn variables(
    vm: &mut Vm,
    p: &mut Paused,
    reference: u32,
) -> Result<Vec<Variable>, String> {
    let Some(target) = p.refs.get(reference.saturating_sub(1) as usize).cloned() else {
        return Err(format!("no variables reference {reference}"));
    };
    let mut out = Vec::new();
    match target {
        RefTarget::Locals(i) => {
            for (name, slot) in vm.frame_locals(i) {
                let s = vm.stack_slot(slot);
                out.push(variable(vm, p, name.to_string(), s));
            }
        }
        RefTarget::Varargs(i) => {
            for (n, s) in vm.frame_varargs(i).into_iter().enumerate() {
                out.push(variable(vm, p, format!("...[{}]", n + 1), s));
            }
        }
        RefTarget::Upvalues(f) => {
            for (name, s) in vm.upvalues(f) {
                out.push(variable(vm, p, name.to_string(), s));
            }
        }
        RefTarget::Table(t) => table_entries(vm, p, t, &mut out),
        RefTarget::Widget(t) => {
            widget_entries(vm, p, Value::Table(t), &mut out);
            table_entries(vm, p, t, &mut out);
        }
    }
    Ok(out)
}

/// A key's sort rank: numbers, then strings, then the rest.
fn key_rank(vm: &mut Vm, k: Value) -> (u8, f64, String) {
    match k {
        Value::Num(n) => (0, n, String::new()),
        Value::Str(_) => (1, 0.0, str_text(vm, k).unwrap_or_default()),
        Value::Bool(b) => (2, f64::from(u8::from(b)), String::new()),
        other => (
            3,
            0.0,
            String::from_utf8_lossy(&vm.raw_tostring(other)).into_owned(),
        ),
    }
}

fn key_text(vm: &mut Vm, k: Value) -> String {
    match k {
        Value::Str(_) => {
            let s = str_text(vm, k).unwrap_or_default();
            let ident = !s.is_empty()
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !s.starts_with(|c: char| c.is_ascii_digit());
            if ident {
                s
            } else {
                format!("[{}]", quoted(&s))
            }
        }
        other => format!("[{}]", describe(vm, other).0),
    }
}

fn table_entries(vm: &mut Vm, p: &mut Paused, t: TableRef, out: &mut Vec<Variable>) {
    let mut entries: Vec<(Value, Slot)> = Vec::new();
    let mut key = Value::Nil;
    while let Ok(Some((k, s))) = vm.heap.table(t).next_slot(key) {
        entries.push((k, s));
        key = k;
        if entries.len() > MAX_ENTRIES {
            break;
        }
    }
    let mut ranked: Vec<((u8, f64, String), Value, Slot)> = entries
        .into_iter()
        .map(|(k, s)| (key_rank(vm, k), k, s))
        .collect();
    ranked.sort_by(|a, b| {
        a.0.0
            .cmp(&b.0.0)
            .then(a.0.1.total_cmp(&b.0.1))
            .then(a.0.2.cmp(&b.0.2))
    });
    let mut entries: Vec<(Value, Slot)> = ranked.into_iter().map(|(_, k, s)| (k, s)).collect();
    let more = entries.len() > MAX_ENTRIES;
    entries.truncate(MAX_ENTRIES);
    for (k, s) in entries {
        let name = key_text(vm, k);
        out.push(variable(vm, p, name, s));
    }
    if more {
        out.push(Variable {
            name: "…".into(),
            value: format!("more than {MAX_ENTRIES} entries"),
            ty: String::new(),
            reference: 0,
            taint: "secure".into(),
            secret: false,
        });
    }
    if let Some(meta) = vm.heap.table(t).meta {
        let s = Slot::secure(Value::Table(meta));
        out.push(variable(vm, p, "[metatable]".into(), s));
    }
}

fn info(name: &str, value: String) -> Variable {
    Variable {
        name: name.into(),
        value,
        ty: String::new(),
        reference: 0,
        taint: "secure".into(),
        secret: false,
    }
}

fn widget_entries(vm: &mut Vm, p: &mut Paused, v: Value, out: &mut Vec<Variable>) {
    let Some(id) = wowlua_engine::widgets::widget_of(vm, v) else {
        return;
    };
    let (class, name, shown, parent, children, points, alpha, scale) = {
        let e = wowlua_engine::engine(vm);
        let w = &e.world.widgets[id.0];
        let points: Vec<String> = w
            .points
            .iter()
            .map(|pt| {
                let rel = match pt
                    .relative
                    .and_then(|r| wowlua_engine::widgets::widget_of(vm, r))
                {
                    Some(r) => wowlua_engine::widgets::widget_label(e, r),
                    None => "parent".into(),
                };
                format!(
                    "{} {rel} {} {} {}",
                    pt.point,
                    pt.relative_point,
                    fmt(pt.x),
                    fmt(pt.y)
                )
            })
            .collect();
        (
            w.class.clone(),
            w.name.clone(),
            w.shown,
            w.parent,
            w.children.clone(),
            points,
            w.alpha,
            w.scale,
        )
    };
    let visible = wowlua_engine::widgets::visible(wowlua_engine::engine(vm), id);
    out.push(info("(type)", class));
    if let Some(n) = name {
        out.push(info("(name)", n));
    }
    out.push(info(
        "(shown)",
        format!(
            "{shown}{}",
            if shown && !visible {
                ", a parent hidden"
            } else {
                ""
            }
        ),
    ));
    if let Some((l, b, w, h)) = wowlua_engine::widgets::bounds(vm, id, 0) {
        out.push(info(
            "(rect)",
            format!(
                "left {} bottom {} width {} height {}",
                fmt(l),
                fmt(b),
                fmt(w),
                fmt(h)
            ),
        ));
    }
    out.push(info("(alpha)", fmt(alpha)));
    out.push(info("(scale)", fmt(scale)));
    for (i, pt) in points.into_iter().enumerate() {
        out.push(info(&format!("(point {})", i + 1), pt));
    }
    if let Some(parent) = parent {
        let t = wowlua_engine::engine(vm).world.widgets[parent.0].table;
        let s = Slot::secure(Value::Table(t));
        out.push(variable(vm, p, "(parent)".into(), s));
    }
    for c in children {
        let t = wowlua_engine::engine(vm).world.widgets[c.0].table;
        let label = wowlua_engine::widgets::widget_label(wowlua_engine::engine(vm), c);
        let s = Slot::secure(Value::Table(t));
        out.push(variable(vm, p, format!("(child {label})"), s));
    }
}

/// The value at a path from the globals (`A.b.c`), through plain tables
/// (raw reads, no metamethods run).
pub(crate) fn lookup_path(vm: &mut Vm, path: &[String]) -> Option<Value> {
    let mut cur = Value::Table(vm.globals);
    for part in path {
        let Value::Table(t) = cur else { return None };
        let k = vm.str(part);
        cur = vm.heap.table(t).get(k);
        if cur.is_nil() {
            return None;
        }
    }
    Some(cur)
}
